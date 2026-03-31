// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bytecode VM.

use alloc::{
    borrow::Cow,
    boxed::Box,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::{fmt, str};

use crate::{
    catalog::{Catalog, read_i32},
    error::{CatalogError, FormatError, HostCallError, Trap},
    schema::decode_opcode_and_next_pc,
    value::{Args, StrId, Value},
};

pub use crate::schema::{
    Decoded, FlowKind, OP_CALL_FUNC, OP_CALL_SELECT, OP_CASE_DEFAULT, OP_CASE_STR,
    OP_EXPR_FALLBACK, OP_HALT, OP_JMP, OP_JMP_IF_FALSE, OP_LOAD_ARG, OP_MARKUP_CLOSE,
    OP_MARKUP_OPEN, OP_OUT_ARG, OP_OUT_EXPR, OP_OUT_LIT, OP_OUT_SLICE, OP_OUT_VAL, OP_PUSH_CONST,
    OP_SELECT_ARG, OP_SELECT_BEGIN, OP_SELECT_END, decode,
};

/// Resolved message handle for repeated formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHandle {
    entry_pc: u32,
}

impl MessageHandle {
    /// Resolve a message id directly from a catalog.
    pub fn from_catalog(catalog: &Catalog, message_id: &str) -> Result<Self, FormatError> {
        let entry_pc = catalog
            .message_pc(message_id)
            .ok_or_else(|| FormatError::UnknownMessageId(message_id.to_string()))?;
        Ok(Self { entry_pc })
    }
}

/// Host callback interface for function calls.
///
/// Implement this trait to provide function behavior for `OP_CALL_FUNC`.
/// The VM also consults [`Host::format_default`] for plain interpolation output.
pub trait Host {
    /// Call function id with positional args and `(key, value)` options.
    fn call(
        &mut self,
        fn_id: u16,
        args: &[Value],
        opts: &[(u32, Value)],
    ) -> Result<Value, HostCallError>;

    /// Call function id for selection (e.g. plural/ordinal category).
    ///
    /// The default delegates to [`call`](Host::call). Hosts may override this
    /// to return `Value::StrRef` for known categories, avoiding allocation.
    fn call_select(
        &mut self,
        fn_id: u16,
        args: &[Value],
        opts: &[(u32, Value)],
    ) -> Result<Value, HostCallError> {
        self.call(fn_id, args, opts)
    }

    /// Optionally format a value for default interpolation.
    ///
    /// Return `Some(String)` to override how this value is rendered by `{ $var }`.
    fn format_default(&mut self, _value: &Value) -> Option<String> {
        None
    }
}

/// Host implementation that always fails unknown functions.
#[derive(Debug, Default)]
pub struct NoopHost;

impl Host for NoopHost {
    fn call(
        &mut self,
        fn_id: u16,
        _args: &[Value],
        _opts: &[(u32, Value)],
    ) -> Result<Value, HostCallError> {
        Err(HostCallError::UnknownFunction { fn_id })
    }
}

/// Sink for structured formatting output.
///
/// Implement this trait to receive rich formatting events instead of flat string output.
/// Used with [`Formatter::format_to`].
///
/// Event ordering matches message execution order. String-oriented sinks usually
/// concatenate [`Self::literal`] and [`Self::expression`] and either ignore or
/// reinterpret markup callbacks. Rich sinks should preserve
/// [`Self::markup_open`] / [`Self::markup_close`] boundaries and associated
/// [`FormatOption`] values.
///
/// Markup does not contribute text on its own. The facade crate's convenience
/// string formatting APIs intentionally discard markup and diagnostics; use the
/// runtime sink path when you need structured output.
pub trait FormatSink {
    /// Literal text from the message pattern.
    fn literal(&mut self, s: &str);
    /// Expression output (variable interpolation, literal expressions, function results).
    fn expression(&mut self, s: &str);
    /// Markup open tag with options.
    ///
    /// Self-closing markup is reported as `markup_open` immediately followed by
    /// `markup_close`.
    fn markup_open(&mut self, name: &str, options: &[FormatOption<'_>]);
    /// Markup close tag with options.
    fn markup_close(&mut self, name: &str, options: &[FormatOption<'_>]);
}

/// One resolved markup option key/value pair delivered to [`FormatSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOption<'a> {
    /// Option key.
    pub key: &'a str,
    /// Option value rendered for sink consumption.
    ///
    /// Literal option values are borrowed from the catalog when possible.
    /// Variable-derived option values may be owned.
    pub value: Cow<'a, str>,
}

trait DiagnosticsSink {
    fn record(&mut self, error: FormatError);
}

#[derive(Default)]
struct VecDiagnostics {
    errors: Vec<FormatError>,
}

impl VecDiagnostics {
    fn into_inner(self) -> Vec<FormatError> {
        self.errors
    }
}

impl DiagnosticsSink for VecDiagnostics {
    fn record(&mut self, error: FormatError) {
        self.errors.push(error);
    }
}

/// Formatter executes catalog messages with caller-provided arguments and host functions.
///
/// Catalogs are expected to come from the compiler or prebuilt assets. This
/// example assumes a loaded catalog whose `"main"` message invokes a host
/// function and formats its result.
///
/// ```rust,no_run
/// use message_format_runtime::{Catalog, FormatError, Formatter, Host, HostCallError, Value};
///
/// #[derive(Default)]
/// struct DemoHost;
///
/// impl Host for DemoHost {
///     fn call(
///         &mut self,
///         _fn_id: u16,
///         _args: &[Value],
///         _opts: &[(u32, Value)],
///     ) -> Result<Value, HostCallError> {
///         Ok(Value::Str("called".to_string()))
///     }
/// }
///
/// # fn render(catalog: &Catalog) -> Result<String, FormatError> {
/// let mut formatter = Formatter::new(&catalog, DemoHost);
/// let message = formatter.resolve("main")?;
/// struct StringSink<'a>(&'a mut String);
/// impl message_format_runtime::FormatSink for StringSink<'_> {
///     fn literal(&mut self, s: &str) { self.0.push_str(s); }
///     fn expression(&mut self, s: &str) { self.0.push_str(s); }
///     fn markup_open(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}
///     fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}
/// }
/// let mut out = String::new();
/// let mut sink = StringSink(&mut out);
/// let _errors = formatter
///     .format_to(message, &Vec::<(u32, Value)>::new(), &mut sink)
///     ?;
/// # Ok(out)
/// # }
/// ```
pub struct Formatter<'a, H: Host> {
    catalog: &'a Catalog,
    host: H,
    fuel: Option<u64>,
    stack: Vec<Value>,
    call_args: Vec<Value>,
    call_options: Vec<(u32, Value)>,
}

enum SelectorValue<'a> {
    Borrowed {
        view: ValueView<'a>,
        str_id: Option<u32>,
    },
    InvalidBorrowed,
    Owned(Value),
}

#[derive(Default)]
struct ExprState {
    fallback_id: Option<u32>,
    pending_errors: Vec<FormatError>,
}

impl<'a> SelectorValue<'a> {
    fn from_borrowed(value: &'a Value, catalog: &'a Catalog) -> Self {
        match value {
            Value::StrRef(id) => {
                catalog
                    .pool_string_opt(*id)
                    .map_or(Self::InvalidBorrowed, |text| Self::Borrowed {
                        view: ValueView::Text(text),
                        str_id: Some(*id),
                    })
            }
            Value::LitRef { off, len } => {
                catalog
                    .literal_opt(*off, *len)
                    .map_or(Self::InvalidBorrowed, |text| Self::Borrowed {
                        view: ValueView::Text(text),
                        str_id: None,
                    })
            }
            _ => Self::Borrowed {
                view: ValueView::from_value(value, catalog)
                    .expect("non-reference values always resolve to a view"),
                str_id: None,
            },
        }
    }

    fn from_stack(stack: &mut Vec<Value>) -> Result<Self, FormatError> {
        stack
            .pop()
            .map(Self::Owned)
            .ok_or(FormatError::StackUnderflow)
    }

    fn matches_case(&self, case_str_id: u32, catalog: &Catalog) -> Result<bool, FormatError> {
        if self.fast_str_id().is_some_and(|id| id == case_str_id) {
            return Ok(true);
        }

        let case = catalog
            .string(case_str_id)
            .map_err(|_| FormatError::Trap(Trap::InvalidCaseStringId))?;
        Ok(match self {
            Self::Borrowed { view, .. } => view.matches_case(case),
            Self::InvalidBorrowed => false,
            Self::Owned(value) => value_matches_case(value, case, catalog),
        })
    }

    fn fast_str_id(&self) -> Option<u32> {
        match self {
            Self::Borrowed { str_id, .. } => *str_id,
            Self::InvalidBorrowed => None,
            Self::Owned(Value::StrRef(id)) => Some(*id),
            Self::Owned(_) => None,
        }
    }
}

impl ExprState {
    fn record_error(&mut self, error: FormatError) {
        self.pending_errors.push(error);
    }

    fn set_fallback(&mut self, fallback_id: u32) {
        self.fallback_id = Some(fallback_id);
    }

    fn should_skip_call(&self) -> bool {
        !self.pending_errors.is_empty()
    }

    fn clear_fallback(&mut self) {
        self.fallback_id = None;
    }

    fn has_fallback(&self) -> bool {
        self.fallback_id.is_some()
    }

    fn push_fallback(
        &mut self,
        stack: &mut Vec<Value>,
        catalog: &Catalog,
    ) -> Result<(), FormatError> {
        push_expr_fallback(stack, catalog, self.fallback_id.take())
    }

    fn take_pending_errors(&mut self) -> Vec<FormatError> {
        core::mem::take(&mut self.pending_errors)
    }

    fn clear_pending_errors(&mut self) {
        self.pending_errors.clear();
    }
}

impl<H: Host> fmt::Debug for Formatter<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Formatter")
            .field("catalog", &self.catalog)
            .finish_non_exhaustive()
    }
}

impl<'a, H: Host> Formatter<'a, H> {
    /// Create a formatter for a loaded catalog.
    #[must_use]
    pub fn new(catalog: &'a Catalog, host: H) -> Self {
        Self {
            catalog,
            host,
            fuel: None,
            stack: Vec::new(),
            call_args: Vec::new(),
            call_options: Vec::new(),
        }
    }

    /// Set the maximum number of instructions the VM may execute per message.
    ///
    /// When the budget is exhausted, formatting returns
    /// [`FormatError::Trap`]. Pass `None` for unlimited execution (the
    /// default). Use this to defend against denial-of-service from untrusted
    /// catalogs that may contain infinite loops.
    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.fuel = fuel;
    }

    /// Resolve a message id to a reusable handle.
    pub fn resolve(&self, message_id: &str) -> Result<MessageHandle, FormatError> {
        MessageHandle::from_catalog(self.catalog, message_id)
    }

    /// Format one message from a previously resolved handle, dispatching events to a [`FormatSink`].
    ///
    /// Returns recoverable formatting diagnostics collected during fallback
    /// rendering. Fatal execution failures are returned as `Err`.
    ///
    /// This is the runtime API that preserves structured markup. In contrast,
    /// string-oriented convenience helpers flatten only literal/expression text
    /// and drop markup events.
    pub fn format_to<S: FormatSink + ?Sized>(
        &mut self,
        message: MessageHandle,
        args: &dyn Args,
        sink: &mut S,
    ) -> Result<Vec<FormatError>, FormatError> {
        let mut diagnostics = VecDiagnostics::default();
        run_bytecode(
            self.catalog,
            &mut self.host,
            message.entry_pc,
            args,
            self.fuel,
            &mut self.stack,
            sink,
            Some(&mut diagnostics),
            &mut self.call_args,
            &mut self.call_options,
        )?;
        Ok(diagnostics.into_inner())
    }
}

fn run_bytecode<S>(
    catalog: &Catalog,
    host: &mut dyn Host,
    entry_pc: u32,
    args: &dyn Args,
    fuel: Option<u64>,
    stack: &mut Vec<Value>,
    sink: &mut S,
    mut diagnostics: Option<&mut dyn DiagnosticsSink>,
    call_args: &mut Vec<Value>,
    call_options: &mut Vec<(u32, Value)>,
) -> Result<(), FormatError>
where
    S: FormatSink + ?Sized,
{
    let code = catalog.code();
    let mut pc = entry_pc;
    stack.clear();
    let mut selector: Option<SelectorValue<'_>> = None;
    let mut expr_state = ExprState::default();
    let mut remaining_fuel = fuel;

    loop {
        if let Some(ref mut f) = remaining_fuel {
            if *f == 0 {
                return Err(FormatError::Trap(Trap::FuelExhausted));
            }
            *f -= 1;
        }

        let (base, opcode, next_pc) = decode_opcode_and_next_pc(code, pc)?;

        match opcode {
            OP_HALT => break,
            OP_JMP => {
                pc = apply_rel_jump(pc, next_pc, read_i32(code, base + 1)?)?;
                continue;
            }
            OP_JMP_IF_FALSE => {
                let value = stack.pop().ok_or(FormatError::StackUnderflow)?;
                if is_falsey(&value, catalog) {
                    pc = apply_rel_jump(pc, next_pc, read_i32(code, base + 1)?)?;
                    continue;
                }
            }
            OP_PUSH_CONST => {
                let id = read_u32(code, base + 1)?;
                if catalog.pool_string_len_opt(id).is_none() {
                    return Err(FormatError::Trap(Trap::InvalidConstStringId));
                }
                stack.push(Value::StrRef(id));
            }
            OP_LOAD_ARG => {
                let id = read_u32(code, base + 1)?;
                let value = load_arg_value(args, catalog, id, &mut expr_state);
                stack.push(value);
            }
            OP_OUT_LIT | OP_OUT_SLICE | OP_OUT_EXPR | OP_OUT_VAL | OP_OUT_ARG => {
                handle_output_instruction(
                    sink,
                    host,
                    stack,
                    catalog,
                    args,
                    &mut diagnostics,
                    opcode,
                    base,
                )?;
            }
            OP_SELECT_ARG | OP_SELECT_BEGIN | OP_CASE_STR | OP_CASE_DEFAULT | OP_SELECT_END => {
                if let Some(jump_pc) = handle_select_instruction(
                    args,
                    stack,
                    &mut selector,
                    catalog,
                    &mut diagnostics,
                    opcode,
                    pc,
                    next_pc,
                    base,
                    code,
                )? {
                    pc = jump_pc;
                    continue;
                }
            }
            OP_EXPR_FALLBACK => {
                expr_state.set_fallback(read_u32(code, base + 1)?);
            }
            OP_CALL_FUNC | OP_CALL_SELECT => {
                handle_call_opcode(
                    host,
                    stack,
                    catalog,
                    opcode,
                    base,
                    code,
                    call_args,
                    call_options,
                    &mut diagnostics,
                    &mut expr_state,
                )?;
            }
            OP_MARKUP_OPEN | OP_MARKUP_CLOSE => {
                handle_markup_instruction(sink, stack, catalog, opcode, base, code, call_options)?;
            }
            _ => return Err(CatalogError::UnknownOpcode { pc, opcode }.into()),
        }

        pc = next_pc;
    }

    Ok(())
}

fn record_diagnostic(diagnostics: &mut Option<&mut dyn DiagnosticsSink>, error: FormatError) {
    if let Some(sink) = diagnostics.as_deref_mut() {
        sink.record(error);
    }
}

fn handle_output_instruction<S>(
    sink: &mut S,
    host: &mut dyn Host,
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    args: &dyn Args,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    opcode: u8,
    base: usize,
) -> Result<(), FormatError>
where
    S: FormatSink + ?Sized,
{
    match opcode {
        OP_OUT_LIT => {
            let id = read_u32(catalog.code(), base + 1)?;
            emit_pool_literal(sink, catalog, id);
        }
        OP_OUT_SLICE => {
            let off = read_u32(catalog.code(), base + 1)?;
            let len = read_u32(catalog.code(), base + 5)?;
            emit_literal_slice(sink, catalog, off, len);
        }
        OP_OUT_EXPR => {
            let off = read_u32(catalog.code(), base + 1)?;
            let len = read_u32(catalog.code(), base + 5)?;
            emit_expr_literal_slice(sink, catalog, off, len);
        }
        OP_OUT_VAL => {
            let value = stack.pop().ok_or(FormatError::StackUnderflow)?;
            emit_output_value(sink, host, catalog, &value);
        }
        OP_OUT_ARG => {
            let key_id = read_u32(catalog.code(), base + 1)?;
            emit_arg_direct_or_fallback(sink, catalog, host, args, key_id, diagnostics);
        }
        _ => return Err(FormatError::Trap(Trap::InvalidOutputOpcode)),
    }

    Ok(())
}

fn handle_select_instruction<'a>(
    args: &'a dyn Args,
    stack: &mut Vec<Value>,
    selector: &mut Option<SelectorValue<'a>>,
    catalog: &'a Catalog,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    opcode: u8,
    pc: u32,
    next_pc: u32,
    base: usize,
    code: &[u8],
) -> Result<Option<u32>, FormatError> {
    match opcode {
        OP_SELECT_ARG => {
            let key_id = read_u32(code, base + 1)?;
            *selector = Some(load_selector_value(args, catalog, key_id, diagnostics));
            Ok(None)
        }
        OP_SELECT_BEGIN => {
            *selector = Some(SelectorValue::from_stack(stack)?);
            Ok(None)
        }
        OP_CASE_STR => {
            let selector = selector
                .as_ref()
                .ok_or(FormatError::Trap(Trap::CaseStringWithoutSelector))?;
            let case_str_id = read_u32(code, base + 1)?;
            if selector.matches_case(case_str_id, catalog)? {
                let rel = read_i32(code, base + 5)?;
                apply_rel_jump(pc, next_pc, rel).map(Some)
            } else {
                Ok(None)
            }
        }
        OP_CASE_DEFAULT => {
            let rel = read_i32(code, base + 1)?;
            apply_rel_jump(pc, next_pc, rel).map(Some)
        }
        OP_SELECT_END => {
            *selector = None;
            Ok(None)
        }
        _ => Err(FormatError::Trap(Trap::InvalidSelectorOpcode)),
    }
}

fn load_selector_value<'a>(
    args: &'a dyn Args,
    catalog: &'a Catalog,
    key_id: StrId,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
) -> SelectorValue<'a> {
    if let Some(value) = args.get_ref(key_id) {
        SelectorValue::from_borrowed(value, catalog)
    } else {
        record_bad_selector(
            diagnostics,
            Some(FormatError::MissingArg(format_id(catalog, key_id))),
        );
        SelectorValue::Owned(Value::Null)
    }
}

fn load_arg_value(
    args: &dyn Args,
    catalog: &Catalog,
    key_id: StrId,
    expr_state: &mut ExprState,
) -> Value {
    match args.get_ref(key_id) {
        Some(v) => v.clone(),
        None => {
            expr_state.record_error(FormatError::MissingArg(format_id(catalog, key_id)));
            Value::Null
        }
    }
}

fn decode_call_operands(
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    arg_count: usize,
    optc: usize,
    call_args: &mut Vec<Value>,
    call_options: &mut Vec<(u32, Value)>,
) -> Result<(), FormatError> {
    decode_option_pairs(stack, catalog, optc, call_options, resolve_call_option_key)?;

    call_args.clear();
    for _ in 0..arg_count {
        call_args.push(stack.pop().ok_or(FormatError::StackUnderflow)?);
    }
    call_args.reverse();

    Ok(())
}

fn decode_option_pairs(
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    optc: usize,
    options: &mut Vec<(u32, Value)>,
    resolve_key: fn(Value, &Catalog) -> Result<u32, FormatError>,
) -> Result<(), FormatError> {
    options.clear();
    for _ in 0..optc {
        let value = stack.pop().ok_or(FormatError::StackUnderflow)?;
        let key = stack.pop().ok_or(FormatError::StackUnderflow)?;
        let key_id = resolve_key(key, catalog)?;
        options.push((key_id, value));
    }
    options.reverse();
    Ok(())
}

fn resolve_call_option_key(key: Value, catalog: &Catalog) -> Result<u32, FormatError> {
    match key {
        Value::Int(id) if id >= 0 => {
            u32::try_from(id).map_err(|_| FormatError::Trap(Trap::CallOptionKeyOutOfRange))
        }
        Value::StrRef(id) => Ok(id),
        Value::Str(name) => catalog
            .string_id(&name)
            .ok_or(FormatError::Trap(Trap::CallOptionKeyUnknown)),
        _ => Err(FormatError::Trap(Trap::CallOptionKeyWrongType)),
    }
}

fn resolve_markup_option_key(key: Value, _catalog: &Catalog) -> Result<u32, FormatError> {
    match key {
        Value::Int(id) if id >= 0 => {
            u32::try_from(id).map_err(|_| FormatError::Trap(Trap::MarkupOptionKeyOutOfRange))
        }
        Value::StrRef(id) => Ok(id),
        _ => Err(FormatError::Trap(Trap::MarkupOptionKeyWrongType)),
    }
}

fn handle_call_opcode(
    host: &mut dyn Host,
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    opcode: u8,
    base: usize,
    code: &[u8],
    call_args: &mut Vec<Value>,
    call_options: &mut Vec<(u32, Value)>,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    expr_state: &mut ExprState,
) -> Result<(), FormatError> {
    let fn_id = read_u16(code, base + 1)?;
    let arg_count = code[base + 3] as usize;
    let optc = code[base + 4] as usize;

    decode_call_operands(stack, catalog, arg_count, optc, call_args, call_options)?;
    handle_call_instruction(
        host,
        opcode,
        fn_id,
        call_args,
        call_options,
        stack,
        catalog,
        diagnostics,
        expr_state,
    )
}

fn handle_markup_instruction<S>(
    sink: &mut S,
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    opcode: u8,
    base: usize,
    code: &[u8],
    call_options: &mut Vec<(u32, Value)>,
) -> Result<(), FormatError>
where
    S: FormatSink + ?Sized,
{
    let name_str_id = read_u32(code, base + 1)?;
    let optc = code[base + 5] as usize;
    decode_option_pairs(
        stack,
        catalog,
        optc,
        call_options,
        resolve_markup_option_key,
    )?;

    if opcode == OP_MARKUP_OPEN {
        emit_markup_open(sink, catalog, name_str_id, call_options);
    } else {
        emit_markup_close(sink, catalog, name_str_id, call_options);
    }

    Ok(())
}

fn handle_call_instruction(
    host: &mut dyn Host,
    opcode: u8,
    fn_id: u16,
    call_args: &[Value],
    call_options: &[(u32, Value)],
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    expr_state: &mut ExprState,
) -> Result<(), FormatError> {
    // If a missing variable was loaded as an operand for this function call,
    // skip the call and use the expression fallback (e.g. `{$varname}`) per
    // TR35 §16.
    if expr_state.should_skip_call() {
        let pending_errors = expr_state.take_pending_errors();
        if opcode == OP_CALL_SELECT {
            let mut pending_errors = pending_errors.into_iter();
            record_bad_selector(diagnostics, pending_errors.next());
            for error in pending_errors {
                record_diagnostic(diagnostics, error);
            }
            expr_state.clear_fallback();
            stack.push(Value::Null);
            return Ok(());
        }
        for error in pending_errors {
            record_diagnostic(diagnostics, error);
        }
        expr_state.push_fallback(stack, catalog)?;
        return Ok(());
    }

    let call_result = if opcode == OP_CALL_SELECT {
        host.call_select(fn_id, call_args, call_options)
    } else {
        host.call(fn_id, call_args, call_options)
    };

    match call_result {
        Ok(result) => {
            expr_state.clear_fallback();
            expr_state.clear_pending_errors();
            stack.push(result);
            Ok(())
        }
        Err(err) => {
            let err = match err {
                HostCallError::UnknownFunction { fn_id } => FormatError::UnknownFunction { fn_id },
                HostCallError::Function(error) => FormatError::Function(error),
            };
            if opcode == OP_CALL_SELECT {
                record_diagnostic(diagnostics, into_bad_selector(err));
                expr_state.clear_fallback();
                expr_state.clear_pending_errors();
                stack.push(Value::Null);
                Ok(())
            } else if expr_state.has_fallback() {
                record_diagnostic(diagnostics, err);
                expr_state.clear_pending_errors();
                expr_state.push_fallback(stack, catalog)
            } else {
                Err(err)
            }
        }
    }
}

fn push_expr_fallback(
    stack: &mut Vec<Value>,
    catalog: &Catalog,
    fallback_id: Option<u32>,
) -> Result<(), FormatError> {
    if let Some(fb_id) = fallback_id {
        catalog
            .string(fb_id)
            .map_err(|_| FormatError::Trap(Trap::InvalidFallbackStringId))?;
        stack.push(Value::StrRef(fb_id));
    } else {
        stack.push(Value::Null);
    }
    Ok(())
}

fn apply_rel_jump(pc: u32, next_pc: u32, rel32: i32) -> Result<u32, FormatError> {
    let target = i64::from(next_pc) + i64::from(rel32);
    if target < 0 {
        return Err(FormatError::BadPc { pc });
    }
    u32::try_from(target).map_err(|_| FormatError::BadPc { pc })
}

fn emit_pool_literal<S: FormatSink + ?Sized>(sink: &mut S, catalog: &Catalog, id: u32) {
    if let Some(text) = catalog.pool_string_opt(id) {
        sink.literal(text);
    }
}

fn emit_literal_slice<S: FormatSink + ?Sized>(sink: &mut S, catalog: &Catalog, off: u32, len: u32) {
    if let Some(text) = catalog.literal_opt(off, len) {
        sink.literal(text);
    }
}

fn emit_expr_literal_slice<S: FormatSink + ?Sized>(
    sink: &mut S,
    catalog: &Catalog,
    off: u32,
    len: u32,
) {
    if let Some(text) = catalog.literal_opt(off, len) {
        sink.expression(text);
    }
}

fn emit_markup_open<S: FormatSink + ?Sized>(
    sink: &mut S,
    catalog: &Catalog,
    name_id: u32,
    options: &[(u32, Value)],
) {
    if let Some(name) = catalog.pool_string_opt(name_id) {
        let resolved = resolve_markup_options(options, catalog);
        sink.markup_open(name, &resolved);
    }
}

fn emit_markup_close<S: FormatSink + ?Sized>(
    sink: &mut S,
    catalog: &Catalog,
    name_id: u32,
    options: &[(u32, Value)],
) {
    if let Some(name) = catalog.pool_string_opt(name_id) {
        let resolved = resolve_markup_options(options, catalog);
        sink.markup_close(name, &resolved);
    }
}

#[derive(Clone, Copy)]
enum ValueView<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'a str),
}

impl<'a> ValueView<'a> {
    fn from_value(value: &'a Value, catalog: &'a Catalog) -> Option<Self> {
        match value {
            Value::Null => Some(Self::Null),
            Value::Bool(v) => Some(Self::Bool(*v)),
            Value::Int(v) => Some(Self::Int(*v)),
            Value::Float(v) => Some(Self::Float(*v)),
            Value::Str(v) => Some(Self::Text(v)),
            Value::StrRef(id) => catalog.pool_string_opt(*id).map(Self::Text),
            Value::LitRef { off, len } => catalog.literal_opt(*off, *len).map(Self::Text),
        }
    }

    fn emit_expression<S: FormatSink + ?Sized>(self, sink: &mut S) {
        match self {
            Self::Null => {}
            Self::Bool(v) => sink.expression(if v { "true" } else { "false" }),
            Self::Int(v) => {
                let rendered = format_i64(v);
                sink.expression(rendered.as_str());
            }
            Self::Float(v) => sink.expression(&v.to_string()),
            Self::Text(v) => sink.expression(v),
        }
    }

    fn format_display(self) -> Cow<'a, str> {
        match self {
            Self::Null => Cow::Borrowed(""),
            Self::Bool(v) => Cow::Borrowed(if v { "true" } else { "false" }),
            Self::Int(v) => Cow::Owned(v.to_string()),
            Self::Float(v) => Cow::Owned(v.to_string()),
            Self::Text(v) => Cow::Borrowed(v),
        }
    }

    fn matches_case(self, case: &str) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(v) => (if v { "true" } else { "false" }) == case,
            Self::Int(v) => int_matches_case(v, case),
            Self::Float(v) => string_value_matches_case(&v.to_string(), case),
            Self::Text(v) => string_value_matches_case(v, case),
        }
    }

    fn is_falsey(self) -> bool {
        match self {
            Self::Null => true,
            Self::Bool(v) => !v,
            Self::Int(v) => v == 0,
            Self::Float(v) => v == 0.0,
            Self::Text(v) => v.is_empty(),
        }
    }
}

fn emit_value_ref<S: FormatSink + ?Sized>(sink: &mut S, catalog: &Catalog, value: &Value) {
    if let Some(view) = ValueView::from_value(value, catalog) {
        view.emit_expression(sink);
    }
}

fn emit_output_value<S: FormatSink + ?Sized>(
    sink: &mut S,
    host: &mut dyn Host,
    catalog: &Catalog,
    value: &Value,
) {
    if let Some(formatted) = host.format_default(value) {
        sink.expression(&formatted);
    } else {
        emit_value_ref(sink, catalog, value);
    }
}

fn emit_arg_direct_or_fallback<S: FormatSink + ?Sized>(
    sink: &mut S,
    catalog: &Catalog,
    host: &mut dyn Host,
    args: &dyn Args,
    key_id: StrId,
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
) {
    if let Some(value) = args.get_ref(key_id) {
        if let Some(formatted) = host.format_default(value) {
            sink.expression(&formatted);
        } else {
            emit_value_ref(sink, catalog, value);
        }
        return;
    }

    // Fallback: render {$varname} per TR35 §16.
    // Record an unresolved-variable error for format_with_diagnostics callers.
    record_missing_arg(diagnostics, catalog, key_id);
    let key = format_id(catalog, key_id);
    let mut fallback = String::with_capacity(key.len() + 3);
    fallback.push('{');
    fallback.push('$');
    fallback.push_str(&key);
    fallback.push('}');
    sink.expression(&fallback);
}

fn resolve_markup_options<'a>(
    options: &'a [(u32, Value)],
    catalog: &'a Catalog,
) -> Vec<FormatOption<'a>> {
    options
        .iter()
        .filter_map(|(key_id, value)| {
            let key = catalog.pool_string_opt(*key_id)?;
            Some(FormatOption {
                key,
                value: format_value_display(value, catalog),
            })
        })
        .collect()
}

fn format_value_display<'a>(value: &'a Value, catalog: &'a Catalog) -> Cow<'a, str> {
    ValueView::from_value(value, catalog).map_or(Cow::Borrowed(""), ValueView::format_display)
}

fn value_matches_case(value: &Value, case: &str, catalog: &Catalog) -> bool {
    ValueView::from_value(value, catalog).is_some_and(|view| view.matches_case(case))
}

fn string_value_matches_case(value: &str, case: &str) -> bool {
    if value == case || truncated_numeric_match(value, case) || percent_one_match(value, case) {
        return true;
    }
    let stripped = strip_bidi_isolates(value);
    stripped.as_ref() == case
        || truncated_numeric_match(stripped.as_ref(), case)
        || percent_one_match(stripped.as_ref(), case)
}

fn strip_bidi_isolates(value: &str) -> Cow<'_, str> {
    if !value.contains(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}']) {
        return Cow::Borrowed(value);
    }
    // The common case has no isolates. Keep that path borrowed so selector
    // matching does not allocate just to confirm there is nothing to strip.
    Cow::Owned(
        value
            .chars()
            .filter(|ch| !matches!(ch, '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{2069}'))
            .collect(),
    )
}

struct IntText {
    buf: [u8; 20],
    start: usize,
}

impl IntText {
    fn as_str(&self) -> &str {
        str::from_utf8(&self.buf[self.start..]).expect("integer formatting is ASCII")
    }
}

fn format_i64(value: i64) -> IntText {
    // Render integers into a fixed stack buffer so direct interpolation and
    // selector matching do not allocate in the VM hot path.
    let mut buf = [0_u8; 20];
    let mut pos = buf.len();
    let mut n = value.unsigned_abs();

    loop {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }

    if value < 0 {
        pos -= 1;
        buf[pos] = b'-';
    }

    IntText { buf, start: pos }
}

fn int_matches_case(value: i64, case: &str) -> bool {
    format_i64(value).as_str() == case
}

fn percent_one_match(value: &str, case: &str) -> bool {
    if case != "one" {
        return false;
    }
    let Some(number) = value.strip_suffix('%') else {
        return false;
    };
    let Ok(parsed) = number.parse::<f64>() else {
        return false;
    };
    parsed == 1.0
}

fn truncated_numeric_match(value: &str, case: &str) -> bool {
    if !is_integer_literal(case) {
        return false;
    }
    let Some(truncated) = truncate_decimal_string(value) else {
        return false;
    };
    truncated == case
}

fn is_integer_literal(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first == '-' {
        return chars.next().is_some_and(|ch| ch.is_ascii_digit())
            && chars.all(|ch| ch.is_ascii_digit());
    }
    first.is_ascii_digit() && chars.all(|ch| ch.is_ascii_digit())
}

fn truncate_decimal_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('e') || trimmed.contains('E') {
        return None;
    }

    let (sign, rest) = if let Some(stripped) = trimmed.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = trimmed.strip_prefix('+') {
        ("", stripped)
    } else {
        ("", trimmed)
    };
    if rest.is_empty() {
        return None;
    }

    let integer = rest.split('.').next()?;
    if integer.is_empty() || !integer.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(format!("{sign}{integer}"))
}

fn is_falsey(value: &Value, catalog: &Catalog) -> bool {
    ValueView::from_value(value, catalog).is_none_or(ValueView::is_falsey)
}

fn format_id(catalog: &Catalog, id: u32) -> String {
    catalog.string(id).unwrap_or("<invalid-id>").to_string()
}

fn record_missing_arg(
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    catalog: &Catalog,
    key_id: StrId,
) {
    record_diagnostic(
        diagnostics,
        FormatError::MissingArg(format_id(catalog, key_id)),
    );
}

fn record_bad_selector(
    diagnostics: &mut Option<&mut dyn DiagnosticsSink>,
    source: Option<FormatError>,
) {
    record_diagnostic(
        diagnostics,
        FormatError::BadSelector {
            source: source.map(Box::new),
        },
    );
}

fn into_bad_selector(error: FormatError) -> FormatError {
    match error {
        bad_selector @ FormatError::BadSelector { .. } => bad_selector,
        source => FormatError::BadSelector {
            source: Some(Box::new(source)),
        },
    }
}

fn read_u16(bytes: &[u8], pos: usize) -> Result<u16, FormatError> {
    let raw = bytes.get(pos..pos + 2).ok_or(FormatError::BadPc {
        pc: bad_pc_from_pos(pos)?,
    })?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], pos: usize) -> Result<u32, FormatError> {
    let raw = bytes.get(pos..pos + 4).ok_or(FormatError::BadPc {
        pc: bad_pc_from_pos(pos)?,
    })?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn bad_pc_from_pos(pos: usize) -> Result<u32, FormatError> {
    u32::try_from(pos).map_err(|_| FormatError::Trap(Trap::ProgramCounterOverflow))
}

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec, vec::Vec};

    use super::*;
    use crate::catalog::{FuncEntry, MessageEntry, build_catalog, build_catalog_with_funcs};
    use crate::error::{ImplementationFailure, MessageFunctionError};

    fn catalog_for_test(strings: &[&str], literals: &str, code: &[u8]) -> Catalog {
        let bytes = if let Some(func_count) = max_function_id(code).map(|id| usize::from(id) + 1) {
            let funcs = (0..func_count)
                .map(|_| FuncEntry {
                    // Tests that only care about call mechanics do not need
                    // distinct function names; reusing pool string 0 keeps the
                    // handcrafted catalog fixture small while satisfying the
                    // verifier's function-table invariant.
                    name_str_id: 0,
                    static_options: vec![],
                })
                .collect::<Vec<_>>();
            build_catalog_with_funcs(
                strings,
                literals,
                &[MessageEntry {
                    name_str_id: 0,
                    entry_pc: 0,
                }],
                code,
                &funcs,
            )
        } else {
            build_catalog(
                strings,
                literals,
                &[MessageEntry {
                    name_str_id: 0,
                    entry_pc: 0,
                }],
                code,
            )
        };
        Catalog::from_bytes(&bytes).expect("valid catalog")
    }

    fn max_function_id(code: &[u8]) -> Option<u16> {
        let mut pc = 0_u32;
        let mut max_fn_id = None;
        while (pc as usize) < code.len() {
            let decoded = decode(code, pc).expect("well-formed test bytecode");
            if decoded.opcode == OP_CALL_FUNC || decoded.opcode == OP_CALL_SELECT {
                let fn_id = u16::from_le_bytes([code[pc as usize + 1], code[pc as usize + 2]]);
                max_fn_id = Some(max_fn_id.map_or(fn_id, |current: u16| current.max(fn_id)));
            }
            pc = decoded.next_pc;
        }
        max_fn_id
    }

    fn formatter_noop(catalog: &Catalog) -> Formatter<'_, NoopHost> {
        Formatter::new(catalog, NoopHost)
    }

    fn arg_id(catalog: &Catalog, name: &str) -> u32 {
        catalog.string_id(name).expect("arg id")
    }

    #[derive(Default)]
    struct TestStringSink {
        out: String,
    }

    impl FormatSink for TestStringSink {
        fn literal(&mut self, s: &str) {
            self.out.push_str(s);
        }

        fn expression(&mut self, s: &str) {
            self.out.push_str(s);
        }

        fn markup_open(&mut self, _name: &str, _options: &[FormatOption<'_>]) {}

        fn markup_close(&mut self, _name: &str, _options: &[FormatOption<'_>]) {}
    }

    trait FormatterTestExt<H: Host> {
        fn format_by_id_for_test(
            &mut self,
            message_id: &str,
            args: &dyn Args,
        ) -> Result<String, FormatError>;
        fn format_to_for_test_by_id(
            &mut self,
            message_id: &str,
            args: &dyn Args,
            sink: &mut dyn FormatSink,
        ) -> Result<Vec<FormatError>, FormatError>;
    }

    impl<H: Host> FormatterTestExt<H> for Formatter<'_, H> {
        fn format_by_id_for_test(
            &mut self,
            message_id: &str,
            args: &dyn Args,
        ) -> Result<String, FormatError> {
            let message = self.resolve(message_id)?;
            let mut sink = TestStringSink::default();
            let _diagnostics = self.format_to(message, args, &mut sink)?;
            Ok(sink.out)
        }

        fn format_to_for_test_by_id(
            &mut self,
            message_id: &str,
            args: &dyn Args,
            sink: &mut dyn FormatSink,
        ) -> Result<Vec<FormatError>, FormatError> {
            let message = self.resolve(message_id)?;
            self.format_to(message, args, sink)
        }
    }

    #[test]
    fn executes_literal_and_arg() {
        let code = vec![
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            6,
            0,
            0,
            0,
            OP_OUT_ARG,
            1,
            0,
            0,
            0,
            OP_OUT_SLICE,
            6,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["hello", "name"], "Hello !", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = vec![(arg_id(&catalog, "name"), Value::Str("World".to_string()))];
        let out = formatter
            .format_by_id_for_test("hello", &args)
            .expect("formatted");
        assert_eq!(out, "Hello World!");
    }

    #[test]
    fn missing_arg_renders_fallback() {
        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let out = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect("formatted");
        assert_eq!(out, "{$name}");
    }

    #[test]
    fn missing_arg_direct_interpolation_records_diagnostic() {
        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = TestStringSink::default();
        let errors = formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .expect("formatted");
        assert_eq!(sink.out, "{$name}");
        assert_eq!(errors, vec![FormatError::MissingArg("name".to_string())]);
    }

    #[test]
    fn missing_arg_in_function_call_records_diagnostic_and_uses_expr_fallback() {
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_EXPR_FALLBACK,
            2,
            0,
            0,
            0,
            OP_CALL_FUNC,
            9,
            0,
            1,
            0,
            OP_OUT_VAL,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "name", "{$name}"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = TestStringSink::default();
        let errors = formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .expect("formatted");
        assert_eq!(sink.out, "{$name}");
        assert_eq!(errors, vec![FormatError::MissingArg("name".to_string())]);
    }

    #[test]
    fn expr_fallback_uses_strref_when_catalog_string_exists() {
        let catalog = catalog_for_test(&["main", "{$name}"], "", &[OP_HALT]);
        let mut stack = Vec::new();

        push_expr_fallback(&mut stack, &catalog, Some(1)).expect("fallback");

        assert_eq!(stack, vec![Value::StrRef(1)]);
    }

    #[test]
    fn missing_selector_records_diagnostic_and_uses_default_arm() {
        let code = vec![
            OP_SELECT_ARG,
            1,
            0,
            0,
            0,
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_JMP,
            9,
            0,
            0,
            0,
            OP_OUT_SLICE,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_SELECT_END,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "sel", "hit"], "HD", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = TestStringSink::default();
        let errors = formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .expect("formatted");
        assert_eq!(sink.out, "D");
        assert_eq!(
            errors,
            vec![FormatError::BadSelector {
                source: Some(Box::new(FormatError::MissingArg("sel".to_string()))),
            }]
        );
    }

    #[test]
    fn selector_call_error_records_bad_selector_and_uses_default_arm() {
        #[derive(Default)]
        struct FailingSelectHost;

        impl Host for FailingSelectHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                panic!("call_select opcode must not dispatch to call()")
            }

            fn call_select(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Err(HostCallError::Function(MessageFunctionError::BadOperand))
            }
        }

        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_CALL_SELECT,
            0,
            0,
            1,
            0,
            OP_SELECT_BEGIN,
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_JMP,
            9,
            0,
            0,
            0,
            OP_OUT_SLICE,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_SELECT_END,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "sel", "hit"], "HD", &code);
        let mut formatter = Formatter::new(&catalog, FailingSelectHost);
        let args = vec![(arg_id(&catalog, "sel"), Value::Int(1))];
        let mut sink = TestStringSink::default();
        let errors = formatter
            .format_to_for_test_by_id("main", &args, &mut sink)
            .expect("formatted");
        assert_eq!(sink.out, "D");
        assert_eq!(
            errors,
            vec![FormatError::BadSelector {
                source: Some(Box::new(FormatError::Function(
                    MessageFunctionError::BadOperand,
                ))),
            }]
        );
    }

    #[test]
    fn legacy_load_arg_select_begin_sequence_still_formats() {
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_SELECT_BEGIN,
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_JMP,
            9,
            0,
            0,
            0,
            OP_OUT_SLICE,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_SELECT_END,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "sel", "hit"], "HD", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = vec![(arg_id(&catalog, "sel"), Value::Str("hit".to_string()))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "H");
    }

    #[test]
    fn unknown_function_is_error_by_default() {
        let code = vec![OP_CALL_FUNC, 7, 0, 0, 0, OP_OUT_VAL, OP_HALT];
        let catalog = catalog_for_test(&["main"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let err = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect_err("must fail");
        assert_eq!(err, FormatError::UnknownFunction { fn_id: 7 });
    }

    #[test]
    fn call_func_preserves_option_count() {
        #[derive(Default)]
        struct HostEchoOptCount;

        impl Host for HostEchoOptCount {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Ok(Value::Str(opts.len().to_string()))
            }
        }

        let code = vec![
            OP_PUSH_CONST,
            1,
            0,
            0,
            0, // key
            OP_PUSH_CONST,
            2,
            0,
            0,
            0, // value
            OP_CALL_FUNC,
            1,
            0,
            0,
            1, // arg_count=0 optc=1
            OP_OUT_VAL,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "k", "v"], "", &code);
        let mut formatter = Formatter::new(&catalog, HostEchoOptCount);
        let out = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect("formatted");
        assert_eq!(out, "1");
    }

    #[test]
    fn format_handle_matches_by_id() {
        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let handle = formatter.resolve("main").expect("resolved");
        let args = vec![(arg_id(&catalog, "name"), Value::Str("Ada".to_string()))];
        let direct = formatter
            .format_by_id_for_test("main", &args)
            .expect("format");
        let mut sink = TestStringSink::default();
        let _diagnostics = formatter
            .format_to(handle, &args, &mut sink)
            .expect("format");
        let resolved = sink.out;
        assert_eq!(direct, resolved);
    }

    #[test]
    fn out_arg_uses_borrowed_lookup_when_available() {
        struct RefOnlyArgs {
            value: Value,
        }

        impl Args for RefOnlyArgs {
            fn get_ref(&self, key: u32) -> Option<&Value> {
                (key == 1).then_some(&self.value)
            }
        }

        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = RefOnlyArgs {
            value: Value::Str("Ada".to_string()),
        };
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "Ada");
    }

    #[test]
    fn string_value_matches_case_ignores_outer_isolates_for_rtl_text() {
        assert!(string_value_matches_case("\u{2067}שלום\u{2069}", "שלום"));
        assert!(string_value_matches_case("\u{2066}مرحبا\u{2069}", "مرحبا"));
    }

    #[test]
    fn string_value_matches_case_ignores_nested_isolates() {
        assert!(string_value_matches_case(
            "\u{2068}אב\u{2067}ג\u{2069}ד\u{2069}",
            "אבגד"
        ));
        assert!(string_value_matches_case(
            "\u{2068}ab\u{2067}cd\u{2069}ef\u{2069}",
            "abcdef"
        ));
    }

    #[test]
    fn string_value_matches_case_does_not_strip_non_isolate_bidi_marks() {
        assert!(!string_value_matches_case("\u{200F}abc", "abc"));
        assert!(!string_value_matches_case("\u{200E}שלום", "שלום"));
    }

    #[test]
    fn strip_bidi_isolates_borrows_when_no_isolates_exist() {
        assert!(matches!(
            strip_bidi_isolates("plain"),
            Cow::Borrowed("plain")
        ));
    }

    #[test]
    fn int_case_matching_handles_negative_numbers_without_string_roundtrip() {
        assert!(int_matches_case(-42, "-42"));
        assert!(!int_matches_case(-42, "42"));
    }

    #[test]
    fn float_nan_and_infinity_format_and_match_consistently() {
        let nan = ValueView::Float(f64::NAN);
        let inf = ValueView::Float(f64::INFINITY);
        let neg_inf = ValueView::Float(f64::NEG_INFINITY);

        let mut sink = TestStringSink::default();
        nan.emit_expression(&mut sink);
        inf.emit_expression(&mut sink);
        neg_inf.emit_expression(&mut sink);
        assert_eq!(
            sink.out,
            format!("{}{}{}", f64::NAN, f64::INFINITY, f64::NEG_INFINITY)
        );

        assert!(nan.matches_case(&f64::NAN.to_string()));
        assert!(inf.matches_case(&f64::INFINITY.to_string()));
        assert!(neg_inf.matches_case(&f64::NEG_INFINITY.to_string()));
    }

    #[test]
    fn percent_one_match_handles_wrapped_percent_output() {
        assert!(percent_one_match("1%", "one"));
        assert!(!percent_one_match("2%", "one"));
    }

    #[test]
    fn truncated_numeric_match_handles_integer_like_strings() {
        assert!(truncated_numeric_match("1.0", "1"));
        assert!(truncated_numeric_match("-42.9", "-42"));
        assert!(!truncated_numeric_match("1.5", "1.5"));
    }

    #[test]
    fn fuel_exhaustion_traps() {
        // Construct a loop that passes the verifier (halt is reachable via
        // the fall-through edge of JMP_IF_FALSE) but always takes the backward
        // branch at runtime because LOAD_ARG for a missing arg pushes Null
        // which is falsey.
        //
        // pc  0: LOAD_ARG "missing"  (5 bytes)  -> pushes Null
        // pc  5: JMP_IF_FALSE rel=-10 (5 bytes) -> Null is falsey, jumps to pc 0
        // pc 10: HALT                 (1 byte)   -> reachable for verifier
        let rel: i32 = -10;
        let rel_bytes = rel.to_le_bytes();
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_JMP_IF_FALSE,
            rel_bytes[0],
            rel_bytes[1],
            rel_bytes[2],
            rel_bytes[3],
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "missing"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        formatter.set_fuel(Some(100));
        let err = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect_err("must trap");
        assert_eq!(err, FormatError::Trap(Trap::FuelExhausted));
    }

    #[test]
    fn fuel_sufficient_succeeds() {
        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        formatter.set_fuel(Some(10));
        let args = vec![(arg_id(&catalog, "name"), Value::Str("Ada".to_string()))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "Ada");
    }

    #[test]
    fn fuel_none_is_unlimited() {
        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        formatter.set_fuel(None);
        let args = vec![(arg_id(&catalog, "name"), Value::Str("Ada".to_string()))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "Ada");
    }

    #[test]
    fn legacy_load_arg_out_val_sequence_still_formats() {
        let code = vec![OP_LOAD_ARG, 1, 0, 0, 0, OP_OUT_VAL, OP_HALT];
        let catalog = catalog_for_test(&["main", "name"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = vec![(arg_id(&catalog, "name"), Value::Str("Ada".to_string()))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "Ada");
    }

    #[test]
    fn call_select_dispatches_to_call_select_method() {
        /// Host where `call` panics but `call_select` returns a `StrRef`.
        #[derive(Default)]
        struct SelectOnlyHost;

        impl Host for SelectOnlyHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                panic!("call_select opcode must not dispatch to call()")
            }

            fn call_select(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                // Return StrRef pointing to "yes" (str_id=2)
                Ok(Value::StrRef(2))
            }
        }

        // String pool: 0="main", 1="x", 2="yes"
        // LOAD_ARG "x", CALL_SELECT fn=0 args=1 opts=0, OUT_VAL, HALT
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_CALL_SELECT,
            0,
            0,
            1,
            0,
            OP_OUT_VAL,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "x", "yes"], "", &code);
        let mut formatter = Formatter::new(&catalog, SelectOnlyHost);
        let args = vec![(arg_id(&catalog, "x"), Value::Int(1))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "yes");
    }

    #[test]
    fn call_select_defaults_to_call_when_host_does_not_override() {
        #[derive(Default)]
        struct DefaultSelectHost;

        impl Host for DefaultSelectHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Ok(Value::Str("delegated".to_string()))
            }
        }

        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_CALL_SELECT,
            0,
            0,
            1,
            0,
            OP_OUT_VAL,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "x"], "", &code);
        let mut formatter = Formatter::new(&catalog, DefaultSelectHost);
        let args = vec![(arg_id(&catalog, "x"), Value::Int(1))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "delegated");
    }

    #[test]
    fn format_default_overrides_plain_interpolation_output() {
        #[derive(Default)]
        struct DefaultFormatHost;

        impl Host for DefaultFormatHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                unreachable!("plain interpolation should only use format_default")
            }

            fn format_default(&mut self, value: &Value) -> Option<String> {
                match value {
                    Value::Int(v) => Some(format!("int:{v}")),
                    _ => None,
                }
            }
        }

        let code = vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main", "n"], "", &code);
        let mut formatter = Formatter::new(&catalog, DefaultFormatHost);
        let args = vec![(arg_id(&catalog, "n"), Value::Int(7))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "int:7");
    }

    #[test]
    fn host_function_error_is_returned_without_expr_fallback() {
        #[derive(Default)]
        struct FailingHost;

        impl Host for FailingHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Err(HostCallError::Function(
                    MessageFunctionError::Implementation(ImplementationFailure::Host),
                ))
            }
        }

        let code = vec![OP_CALL_FUNC, 0, 0, 0, 0, OP_OUT_VAL, OP_HALT];
        let catalog = catalog_for_test(&["main"], "", &code);
        let mut formatter = Formatter::new(&catalog, FailingHost);
        let err = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect_err("must fail");
        assert_eq!(
            err,
            FormatError::Function(MessageFunctionError::Implementation(
                ImplementationFailure::Host,
            ))
        );
    }

    #[test]
    fn case_str_fast_path_matches_strref_by_id() {
        /// Host that returns StrRef(2) which is "alpha" in the pool.
        #[derive(Default)]
        struct StrRefHost;

        impl Host for StrRefHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Ok(Value::StrRef(2))
            }
        }

        // String pool: 0="main", 1="x", 2="alpha", 3="beta"
        // Bytecode: LOAD_ARG "x", CALL_FUNC fn=0 args=1 opts=0,
        //           SELECT_BEGIN,
        //           CASE_STR "alpha"(2) -> hit branch,
        //           CASE_DEFAULT -> miss branch,
        //           hit: OUT_SLICE "HIT", JMP end,
        //           miss: OUT_SLICE "MISS",
        //           SELECT_END, HALT
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0, // pc 0
            OP_CALL_FUNC,
            0,
            0,
            1,
            0,               // pc 5
            OP_SELECT_BEGIN, // pc 10
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0, // pc 11, jump +5 -> pc 25
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0, // pc 20, jump +14 -> pc 39
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            3,
            0,
            0,
            0, // pc 25, "HIT"
            OP_JMP,
            9,
            0,
            0,
            0, // pc 34, jump +9 -> pc 48
            OP_OUT_SLICE,
            3,
            0,
            0,
            0,
            4,
            0,
            0,
            0,             // pc 39, "MISS"
            OP_SELECT_END, // pc 48
            OP_HALT,       // pc 49
        ];
        let catalog = catalog_for_test(&["main", "x", "alpha", "beta"], "HITMISS", &code);
        let mut formatter = Formatter::new(&catalog, StrRefHost);
        let args = vec![(arg_id(&catalog, "x"), Value::Int(1))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "HIT");
    }

    #[test]
    fn case_str_strref_mismatch_falls_through_to_string_compare() {
        /// Host that returns StrRef(3) which is "beta", not matching "alpha"(2).
        #[derive(Default)]
        struct StrRefMissHost;

        impl Host for StrRefMissHost {
            fn call(
                &mut self,
                _fn_id: u16,
                _args: &[Value],
                _opts: &[(u32, Value)],
            ) -> Result<Value, HostCallError> {
                Ok(Value::StrRef(3))
            }
        }

        // Same layout as above but host returns StrRef(3)="beta",
        // CASE_STR checks for "alpha"(2) — ID mismatch, falls to default.
        let code = vec![
            OP_LOAD_ARG,
            1,
            0,
            0,
            0,
            OP_CALL_FUNC,
            0,
            0,
            1,
            0,
            OP_SELECT_BEGIN,
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            3,
            0,
            0,
            0,
            OP_JMP,
            9,
            0,
            0,
            0,
            OP_OUT_SLICE,
            3,
            0,
            0,
            0,
            4,
            0,
            0,
            0,
            OP_SELECT_END,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "x", "alpha", "beta"], "HITMISS", &code);
        let mut formatter = Formatter::new(&catalog, StrRefMissHost);
        let args = vec![(arg_id(&catalog, "x"), Value::Int(1))];
        let out = formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted");
        assert_eq!(out, "MISS");
    }

    /// Collecting sink for testing `FormatSink`.
    #[derive(Debug, Default)]
    struct CollectingSink {
        events: Vec<SinkEvent>,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum SinkEvent {
        Literal(String),
        Expression(String),
        MarkupOpen(String, Vec<(String, String)>),
        MarkupClose(String, Vec<(String, String)>),
    }

    impl FormatSink for CollectingSink {
        fn literal(&mut self, s: &str) {
            self.events.push(SinkEvent::Literal(s.to_string()));
        }
        fn expression(&mut self, s: &str) {
            self.events.push(SinkEvent::Expression(s.to_string()));
        }
        fn markup_open(&mut self, name: &str, options: &[FormatOption<'_>]) {
            self.events.push(SinkEvent::MarkupOpen(
                name.to_string(),
                options
                    .iter()
                    .map(|option| (option.key.to_string(), option.value.to_string()))
                    .collect(),
            ));
        }
        fn markup_close(&mut self, name: &str, options: &[FormatOption<'_>]) {
            self.events.push(SinkEvent::MarkupClose(
                name.to_string(),
                options
                    .iter()
                    .map(|option| (option.key.to_string(), option.value.to_string()))
                    .collect(),
            ));
        }
    }

    #[test]
    fn markup_open_close_produce_no_string_output() {
        // OP_MARKUP_OPEN "b" (str_id=1) optc=0, OP_MARKUP_CLOSE "b" optc=0
        let code = vec![
            OP_MARKUP_OPEN,
            1,
            0,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            2,
            0,
            0,
            0,
            OP_MARKUP_CLOSE,
            1,
            0,
            0,
            0,
            0,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "b"], "hi", &code);
        let mut formatter = formatter_noop(&catalog);
        let out = formatter
            .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
            .expect("formatted");
        assert_eq!(out, "hi");
    }

    #[test]
    fn out_expr_produces_same_string_as_out_slice() {
        // OP_OUT_EXPR uses same encoding as OP_OUT_SLICE
        let code_expr = vec![OP_OUT_EXPR, 0, 0, 0, 0, 5, 0, 0, 0, OP_HALT];
        let code_slice = vec![OP_OUT_SLICE, 0, 0, 0, 0, 5, 0, 0, 0, OP_HALT];
        let catalog_expr = catalog_for_test(&["main"], "Hello", &code_expr);
        let catalog_slice = catalog_for_test(&["main"], "Hello", &code_slice);
        let mut fmt_expr = formatter_noop(&catalog_expr);
        let mut fmt_slice = formatter_noop(&catalog_slice);
        let args = Vec::<(u32, Value)>::new();
        assert_eq!(
            fmt_expr.format_by_id_for_test("main", &args).unwrap(),
            fmt_slice.format_by_id_for_test("main", &args).unwrap()
        );
    }

    #[test]
    fn format_to_sink_receives_correct_events() {
        // "Hello " (literal text) + $name (expression) + markup open/close around it
        // MARKUP_OPEN "b" optc=0, OUT_SLICE "Hello ", LOAD_ARG "name", OUT_VAL, MARKUP_CLOSE "b" optc=0
        let code = vec![
            OP_MARKUP_OPEN,
            1,
            0,
            0,
            0,
            0, // {#b}
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            6,
            0,
            0,
            0, // "Hello "
            OP_LOAD_ARG,
            2,
            0,
            0,
            0,          // $name
            OP_OUT_VAL, // output
            OP_MARKUP_CLOSE,
            1,
            0,
            0,
            0,
            0, // {/b}
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "b", "name"], "Hello ", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = vec![(arg_id(&catalog, "name"), Value::Str("World".to_string()))];
        let mut sink = CollectingSink::default();
        let errors = formatter
            .format_to_for_test_by_id("main", &args, &mut sink)
            .unwrap();
        assert!(errors.is_empty());
        assert_eq!(
            sink.events,
            vec![
                SinkEvent::MarkupOpen("b".to_string(), vec![]),
                SinkEvent::Literal("Hello ".to_string()),
                SinkEvent::Expression("World".to_string()),
                SinkEvent::MarkupClose("b".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn markup_options_with_literal_values_resolve_in_sink() {
        // PUSH_CONST "href"(2), PUSH_CONST "https://example.com"(3),
        // MARKUP_OPEN "a"(1) optc=1
        let code = vec![
            OP_PUSH_CONST,
            2,
            0,
            0,
            0,
            OP_PUSH_CONST,
            3,
            0,
            0,
            0,
            OP_MARKUP_OPEN,
            1,
            0,
            0,
            0,
            1,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            4,
            0,
            0,
            0,
            OP_MARKUP_CLOSE,
            1,
            0,
            0,
            0,
            0,
            OP_HALT,
        ];
        let catalog =
            catalog_for_test(&["main", "a", "href", "https://example.com"], "link", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = CollectingSink::default();
        formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .unwrap();
        assert_eq!(
            sink.events,
            vec![
                SinkEvent::MarkupOpen(
                    "a".to_string(),
                    vec![("href".to_string(), "https://example.com".to_string()),]
                ),
                SinkEvent::Literal("link".to_string()),
                SinkEvent::MarkupClose("a".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn markup_options_with_variable_values_resolve_in_sink() {
        // PUSH_CONST "href"(2), LOAD_ARG "url"(3),
        // MARKUP_OPEN "a"(1) optc=1
        let code = vec![
            OP_PUSH_CONST,
            2,
            0,
            0,
            0,
            OP_LOAD_ARG,
            3,
            0,
            0,
            0,
            OP_MARKUP_OPEN,
            1,
            0,
            0,
            0,
            1,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_MARKUP_CLOSE,
            1,
            0,
            0,
            0,
            0,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "a", "href", "url"], "click", &code);
        let mut formatter = formatter_noop(&catalog);
        let args = vec![(
            arg_id(&catalog, "url"),
            Value::Str("https://test.com".to_string()),
        )];
        let mut sink = CollectingSink::default();
        formatter
            .format_to_for_test_by_id("main", &args, &mut sink)
            .unwrap();
        assert_eq!(
            sink.events,
            vec![
                SinkEvent::MarkupOpen(
                    "a".to_string(),
                    vec![("href".to_string(), "https://test.com".to_string()),]
                ),
                SinkEvent::Literal("click".to_string()),
                SinkEvent::MarkupClose("a".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn self_closing_markup_produces_open_close_events() {
        // MARKUP_OPEN "br"(1) optc=0, MARKUP_CLOSE "br"(1) optc=0
        let code = vec![
            OP_MARKUP_OPEN,
            1,
            0,
            0,
            0,
            0,
            OP_MARKUP_CLOSE,
            1,
            0,
            0,
            0,
            0,
            OP_HALT,
        ];
        let catalog = catalog_for_test(&["main", "br"], "", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = CollectingSink::default();
        formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .unwrap();
        assert_eq!(
            sink.events,
            vec![
                SinkEvent::MarkupOpen("br".to_string(), vec![]),
                SinkEvent::MarkupClose("br".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn out_expr_dispatches_as_expression_event() {
        let code = vec![OP_OUT_EXPR, 0, 0, 0, 0, 5, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main"], "hello", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = CollectingSink::default();
        formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .unwrap();
        assert_eq!(
            sink.events,
            vec![SinkEvent::Expression("hello".to_string())]
        );
    }

    #[test]
    fn out_slice_dispatches_as_literal_event() {
        let code = vec![OP_OUT_SLICE, 0, 0, 0, 0, 5, 0, 0, 0, OP_HALT];
        let catalog = catalog_for_test(&["main"], "hello", &code);
        let mut formatter = formatter_noop(&catalog);
        let mut sink = CollectingSink::default();
        formatter
            .format_to_for_test_by_id("main", &Vec::<(u32, Value)>::new(), &mut sink)
            .unwrap();
        assert_eq!(sink.events, vec![SinkEvent::Literal("hello".to_string())]);
    }
}
