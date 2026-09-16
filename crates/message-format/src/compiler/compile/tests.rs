// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::runtime::BuiltinHost;
use crate::runtime::schema;
use crate::runtime::{
    Catalog, Formatter, FunctionOptions, Host, HostCallError, HostFn, MessageFunctionError,
    NoopHost, Value,
};
use alloc::rc::Rc;
use core::cell::Cell;

use crate::compiler::manifest::{
    FunctionManifest, FunctionOperandKind, FunctionOptionValueKind, FunctionSchema,
    FunctionSelectorKeyKind,
};
use crate::compiler::resource::{MessageResource, ResourceInput, ResourceSpan};
use crate::compiler::semantic::{DetachedSourceSpan, OperandLiteralKind, SelectorExpr};
use crate::compiler::test_support::FormatterTestExt;

use super::frontend::{lower_expression_node_to_part, parse_messages};
use super::*;

fn expect_compiled(report: CompileReport) -> CompiledCatalog {
    assert!(!report.has_errors(), "{}", report.render());
    report.compiled.expect("compiled catalog")
}

fn expect_first_error(report: CompileReport) -> BuildError {
    assert!(report.has_errors(), "expected compile errors");
    report
        .diagnostics
        .into_iter()
        .next()
        .expect("first diagnostic")
}

fn expect_errors(report: CompileReport) -> Vec<BuildError> {
    assert!(report.has_errors(), "expected compile errors");
    report.diagnostics
}

fn passthrough_host() -> impl Host<CatalogIndex = ()> {
    HostFn(|_, args: &[Value], _: FunctionOptions<'_>| {
        Ok(args.first().cloned().unwrap_or(Value::Null))
    })
}

fn arg_id(catalog: &Catalog, name: &str) -> u32 {
    catalog.string_id(name).expect("arg id")
}

fn arg(catalog: &Catalog, name: &str, value: Value) -> (u32, Value) {
    (arg_id(catalog, name), value)
}

fn opcodes(catalog: &Catalog) -> Vec<schema::Opcode> {
    let mut result = Vec::new();
    let mut pc = 0;
    while pc < catalog.code().len() {
        let opcode = schema::Opcode::try_from(catalog.code()[pc]).expect("valid opcode");
        result.push(opcode);
        pc += opcode.bytes();
    }
    result
}

fn chunk_len(bytes: &[u8], tag: [u8; 4]) -> u32 {
    let chunk_count = u32::from_le_bytes(bytes[16..20].try_into().expect("chunk count"));
    let chunk_table_offset =
        u32::from_le_bytes(bytes[20..24].try_into().expect("chunk table offset")) as usize;
    for idx in 0..chunk_count as usize {
        let pos = chunk_table_offset + idx * 16;
        if bytes[pos..pos + 4] == tag {
            return u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().expect("chunk len"));
        }
    }
    panic!("missing chunk {tag:?}");
}

#[derive(Default)]
struct MarkupOptionSink {
    options: Vec<(String, String)>,
}

impl crate::runtime::FormatSink for MarkupOptionSink {
    fn literal(&mut self, _s: &str) {}

    fn expression(&mut self, _s: &str) {}

    fn markup_open(&mut self, _name: &str, options: &[crate::runtime::FormatOption<'_>]) {
        self.options.extend(
            options
                .iter()
                .map(|option| (option.key.to_string(), option.value.to_string())),
        );
    }

    fn markup_close(&mut self, _name: &str, _options: &[crate::runtime::FormatOption<'_>]) {}
}

#[derive(Default)]
struct UniversalIdSink {
    output: String,
    ids: Vec<String>,
}

impl crate::runtime::FormatSink for UniversalIdSink {
    fn wants_structured_output(&self) -> bool {
        true
    }

    fn literal(&mut self, value: &str) {
        self.output.push_str(value);
    }

    fn expression(&mut self, value: &str) {
        self.output.push_str(value);
    }

    fn markup_open(&mut self, _name: &str, _options: &[crate::runtime::FormatOption<'_>]) {}

    fn markup_close(&mut self, _name: &str, _options: &[crate::runtime::FormatOption<'_>]) {}

    fn formatted_value(&mut self, value: &crate::runtime::FormattedValue<'_>) {
        self.output.push_str(&value.value);
        if let Some(id) = value.id {
            self.ids.push(id.to_string());
        }
    }
}

#[test]
fn bare_interpolation_lowers_to_out_arg() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert_eq!(
        catalog.code(),
        &[
            schema::Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            6,
            0,
            0,
            0,
            schema::Opcode::OutArg as u8,
            1,
            0,
            0,
            0,
            schema::Opcode::OutSlice as u8,
            6,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
        ]
    );
}

#[test]
fn bare_string_input_selects_without_a_function_host() {
    let bytes =
        compile_str(".input { $kind :string }\n.match $kind\nformal {{Good evening}}\n* {{Hi}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let code = opcodes(&catalog);
    assert_eq!(code.first(), Some(&schema::Opcode::LoadArg));
    assert!(code.contains(&schema::Opcode::ResolveString));
    assert!(code.contains(&schema::Opcode::StoreLocal));
    assert!(code.contains(&schema::Opcode::SelectLocal));
    assert!(!code.contains(&schema::Opcode::CallFunc));

    let args = vec![arg(&catalog, "kind", Value::Str("formal".to_string()))];
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "Good evening"
    );
}

#[test]
fn bare_string_input_uses_exact_text_for_non_string_values() {
    let bytes =
        compile_str(".input {$x :string} .match $x 1 {{integer}} 1.0 {{decimal}} * {{other}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let x = catalog.string_id("x").expect("argument id");
    let decimal = catalog.string_id("1.0").expect("decimal key id");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &[(x, Value::Float(1.5))])
            .expect("float selected"),
        "other"
    );
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &[(x, Value::StrRef(decimal))])
            .expect("pool string selected"),
        "decimal"
    );
}

#[test]
fn bare_string_input_selects_present_null_as_empty_text() {
    for source in [
        ".input {$x :string} .match $x || {{EMPTY}} * {{OTHER}}",
        ".input {$x :string} .match $x $x || || {{EMPTY}} * * {{OTHER}}",
    ] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let args = vec![arg(&catalog, "x", Value::Null)];
        let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

        assert_eq!(
            formatter
                .format_by_id_for_test("main", &args)
                .expect("formatted"),
            "EMPTY",
            "source={source}"
        );
    }
}

#[test]
fn structured_optionless_string_selector_resolves_literal_exactly() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("main")
                .select(
                    SelectExpr::builder(SelectorExpr::call(
                        Operand::literal("1.5"),
                        FunctionSpec::new("string"),
                    ))
                    .arm("1", vec![Part::text("WRONG")])
                    .arm("1.5", vec![Part::text("EXACT")])
                    .default(vec![Part::text("OTHER")])
                    .build(),
                )
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert!(opcodes(&catalog).contains(&schema::Opcode::ResolveString));
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &[])
            .expect("formatted"),
        "EXACT"
    );
}

#[test]
fn repeated_bare_string_selector_does_not_require_a_host() {
    let bytes = compile_str(".input {$kind :string} .match $kind $kind a a {{A}} * * {{OTHER}}")
        .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "kind", Value::Str("a".to_string()))];
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "A");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn bare_string_selector_is_host_free_when_a_fallback_arm_reads_the_input() {
    let bytes =
        compile_str(".input {$kind :string} .match $kind a {{A}} * {{{$kind}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "kind", Value::Str("a".to_string()))];
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "A");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn missing_bare_string_input_reports_once_before_selector_validation() {
    let bytes = compile_str(".input {$x :string} .match $x a {{A}} * {{{$x}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(
            message,
            &Vec::<(u32, Value)>::new(),
            &mut output,
            Some(&mut diagnostics),
        )
        .expect("formatted");

    assert_eq!(output, "{$x}");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(matches!(
        diagnostics[0],
        crate::runtime::FormatError::MissingArg(ref name) if name == "x"
    ));
}

#[test]
fn unused_string_input_is_still_resolved_once() {
    let bytes = compile_str(".input {$x :string} {{hello}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(
            message,
            &Vec::<(u32, Value)>::new(),
            &mut output,
            Some(&mut diagnostics),
        )
        .expect("formatted");

    assert_eq!(output, "hello");
    assert_eq!(
        diagnostics,
        vec![crate::runtime::FormatError::MissingArg("x".to_string())]
    );
}

#[test]
fn string_selector_retries_do_not_repeat_declaration_diagnostics() {
    let source = ".input {$x :number} .input {$y :string} .match $x $y \
                  1 a {{EXACT}} one b {{PLURAL}} * * {{OTHER}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "OTHER");
    assert_eq!(
        diagnostics,
        vec![crate::runtime::FormatError::MissingArg("y".to_string())]
    );
}

#[test]
fn string_input_diagnostic_precedes_later_declaration_diagnostic() {
    let source = ".input {$x :string} .local $y = {$missing :number} \
                  .match $x a {{A}} * {{OTHER}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(
            message,
            &Vec::<(u32, Value)>::new(),
            &mut output,
            Some(&mut diagnostics),
        )
        .expect("formatted");

    assert_eq!(output, "OTHER");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::MissingArg("x".to_string()),
            crate::runtime::FormatError::MissingArg("missing".to_string()),
            crate::runtime::FormatError::Function(MessageFunctionError::BadOperand),
        ]
    );
}

#[test]
fn bare_string_resolution_is_not_locale_number_formatting() {
    let bytes =
        compile_str(".input {$x :string} .match $x 1.5 {{MATCH}} * {{OTHER}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "fr".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![arg(&catalog, "x", Value::Float(1.5))];

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "MATCH"
    );
}

#[test]
fn single_use_string_to_number_chain_avoids_intermediate_string_storage() {
    let source = ".input {$x :string} .local $n = {$x :number} {{value={$n}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let emitted = opcodes(&catalog);

    assert!(emitted.contains(&schema::Opcode::ResolveString));
    assert_eq!(
        emitted
            .iter()
            .filter(|opcode| **opcode == schema::Opcode::StoreLocal)
            .count(),
        1
    );

    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![arg(&catalog, "x", Value::Int(21))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "value=21"
    );
}

#[test]
fn fused_string_to_number_resolves_the_producers_operand() {
    for source in [
        ".local $n = {42} .local $s = {$n :string} .local $x = {$s :number} {{{$x}}}",
        ".local $n = {42 :number} .local $s = {$n :string} .local $x = {$s :number} {{{$x}}}",
    ] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let mut formatter =
            Formatter::new(&catalog, BuiltinHost::new(&locale).expect("host")).expect("formatter");
        let message = formatter.resolve("main").expect("message");
        let mut output = String::new();
        let mut diagnostics = Vec::new();

        formatter
            .format_to(message, &[], &mut output, Some(&mut diagnostics))
            .expect("formatted");

        assert_eq!(output, "42", "source={source}");
        assert!(diagnostics.is_empty(), "source={source}: {diagnostics:?}");
    }
}

#[test]
fn fused_string_to_number_evaluates_its_input_once() {
    let source = ".input {$n :custom} .local $s = {$n :string} .local $x = {$s :number} {{{$x}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(move |_, args: &[Value], _| {
            observed.set(observed.get() + 1);
            Ok(args.first().cloned().unwrap_or(Value::Null))
        }),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "n", Value::Int(42))];

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "42"
    );
    assert_eq!(calls.get(), 2);
}

#[test]
fn ordinary_body_references_prevent_single_use_string_fusion() {
    let source = ".input {$s :string} .local $n = {$s :number} {{{$s} {$s} {$n}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let mut formatter =
        Formatter::new(&catalog, BuiltinHost::new(&locale).expect("host")).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &[], &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(
        diagnostics
            .iter()
            .filter(|error| matches!(error, crate::runtime::FormatError::MissingArg(name) if name == "s"))
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[derive(Default)]
struct SplitSelectionHost;

impl Host for SplitSelectionHost {
    type CatalogIndex = ();

    fn index(
        &mut self,
        _catalog: &Catalog,
    ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
        Ok(())
    }

    fn call(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Ok(Value::Str("display".to_string()))
    }

    fn call_select(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Ok(Value::Str("one".to_string()))
    }
}

#[derive(Default)]
struct FailingSelectionHost;

impl Host for FailingSelectionHost {
    type CatalogIndex = ();

    fn index(
        &mut self,
        _catalog: &Catalog,
    ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
        Ok(())
    }

    fn call(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Ok(Value::Str("display".to_string()))
    }

    fn call_select(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Err(HostCallError::Function(MessageFunctionError::BadOperand))
    }
}

#[derive(Default)]
struct FailingNumericResolutionHost;

impl Host for FailingNumericResolutionHost {
    type CatalogIndex = ();

    fn index(
        &mut self,
        _catalog: &Catalog,
    ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
        Ok(())
    }

    fn call(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Err(HostCallError::Function(MessageFunctionError::BadOption))
    }

    fn call_select(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Ok(Value::Str("one".to_string()))
    }
}

#[derive(Default)]
struct RecoveringSelectionHost;

impl Host for RecoveringSelectionHost {
    type CatalogIndex = ();

    fn index(
        &mut self,
        _catalog: &Catalog,
    ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
        Ok(())
    }

    fn call(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        args: &[Value],
        _opts: FunctionOptions<'_>,
        _on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        Ok(args[0].clone())
    }

    fn call_select(
        &mut self,
        _catalog: &Catalog,
        _index: &Self::CatalogIndex,
        _fn_id: u16,
        _args: &[Value],
        _opts: FunctionOptions<'_>,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        on_error(MessageFunctionError::BadOption);
        Ok(Value::Str("one".to_string()))
    }
}

#[test]
fn stored_custom_input_uses_the_hosts_selection_entry_point() {
    let bytes =
        compile_str(".input {$x :custom} .match $x one {{ONE}} * {{OTHER}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    let mut formatter = Formatter::new(&catalog, SplitSelectionHost).expect("formatter");

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "ONE"
    );
}

#[test]
fn numeric_input_selection_preserves_custom_host_resolution_failure() {
    let bytes =
        compile_str(".input {$n :number} .match $n one {{ONE}} * {{OTHER}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "n", Value::Int(1))];
    let mut formatter = Formatter::new(&catalog, FailingNumericResolutionHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "OTHER");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::Function(MessageFunctionError::BadOption),
            crate::runtime::FormatError::BadSelector { source: None },
        ]
    );
}

#[test]
fn stored_custom_selector_errors_collapse_to_one_bad_selector() {
    let bytes =
        compile_str(".input {$x :custom} .match $x one {{ONE}} * {{OTHER}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    let mut formatter = Formatter::new(&catalog, FailingSelectionHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "OTHER");
    assert_eq!(
        diagnostics,
        vec![crate::runtime::FormatError::BadSelector { source: None }]
    );
}

#[test]
fn stored_custom_selector_keeps_recoverable_host_diagnostics() {
    let bytes =
        compile_str(".input {$x :custom} .match $x one {{ONE}} * {{OTHER}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    let mut formatter = Formatter::new(&catalog, RecoveringSelectionHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "ONE");
    assert_eq!(
        diagnostics,
        vec![crate::runtime::FormatError::Function(
            MessageFunctionError::BadOption
        )]
    );
}

#[test]
fn string_input_selector_preserves_eager_declaration_slots() {
    let source =
        ".input {$kind :string} .input {$n :number} .match $kind formal {{n={$n}}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let code = opcodes(&catalog);
    assert_eq!(
        code.iter()
            .filter(|opcode| **opcode == schema::Opcode::StoreLocal)
            .count(),
        2
    );

    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![
        arg(&catalog, "kind", Value::Str("formal".to_string())),
        arg(&catalog, "n", Value::Int(7)),
    ];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "n=7"
    );
}

#[test]
fn runtime_value_alias_does_not_consume_a_slot() {
    let source = ".input {$n :number} .local $alias = {$n} {{n={$alias}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert_eq!(
        opcodes(&catalog)
            .iter()
            .filter(|opcode| **opcode == schema::Opcode::StoreLocal)
            .count(),
        1
    );
}

#[test]
fn runtime_value_alias_in_dynamic_option_uses_source_slot() {
    let source = ".input {$digits :number} .local $alias = {$digits} \
                  {{{$n :number minimumFractionDigits=$alias}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![
        arg(&catalog, "digits", Value::Int(2)),
        arg(&catalog, "n", Value::Int(3)),
    ];

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "3.00"
    );
}

#[test]
fn unused_implicit_argument_alias_remains_eager() {
    let bytes = compile_str(".local $alias = {$missing} {{hello}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &[], &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "hello");
    assert_eq!(
        diagnostics,
        vec![crate::runtime::FormatError::MissingArg(
            "missing".to_string()
        )]
    );
}

#[test]
fn stored_numeric_keyword_projection_reuses_declaration_function_entry() {
    let source = ".input {$n :number minimumFractionDigits=2} .match $n one {{one}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");

    assert_eq!(catalog.func_count(), 1);
    let emitted = opcodes(&catalog);
    assert!(emitted.contains(&schema::Opcode::CallFunc));
    assert!(emitted.contains(&schema::Opcode::StoreLocal));
    assert!(emitted.contains(&schema::Opcode::ProjectSelect));
    assert!(!emitted.contains(&schema::Opcode::CallSelect));
}

#[test]
fn structured_numeric_selector_calls_apply_the_current_annotation() {
    for (stored, selector, key) in [
        ("1.5", FunctionSpec::new("integer"), "one"),
        (
            "2",
            FunctionSpec::new("number").option_literal("select", "ordinal"),
            "two",
        ),
        (
            "1",
            FunctionSpec::new("number").option_literal("minimumFractionDigits", "2"),
            "other",
        ),
        (
            "0",
            FunctionSpec::new("offset").option_literal("add", "1"),
            "one",
        ),
    ] {
        let mut builder = CatalogBuilder::new();
        builder
            .add_message(
                Message::builder("main")
                    .part(Part::Bind {
                        slot: 0,
                        fallback: String::from("{$n}"),
                        value: Box::new(Part::call(CallExpr::new(
                            Operand::number_literal(stored),
                            FunctionSpec::new("number"),
                        ))),
                    })
                    .select(
                        SelectExpr::builder(SelectorExpr::call(Operand::Local(0), selector))
                            .arm(key, vec![Part::text("MATCH")])
                            .default(vec![Part::text("OTHER")])
                            .build(),
                    )
                    .build(),
            )
            .expect("message");
        let compiled = expect_compiled(builder.compile());
        let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");

        assert_eq!(
            formatter
                .format_by_id_for_test("main", &[])
                .expect("formatted"),
            "MATCH",
            "stored={stored}, key={key}"
        );
    }
}

#[test]
fn structured_string_selector_matches_formatted_presentation() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("main")
                .select(
                    SelectExpr::builder(SelectorExpr::call(
                        Operand::var("v"),
                        FunctionSpec::new("string"),
                    ))
                    .arm("1%", vec![Part::text("MATCH")])
                    .default(vec![Part::text("OTHER")])
                    .build(),
                )
                .build(),
        )
        .expect("message");
    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let value = Value::Formatted(Box::new(crate::runtime::ResolvedFormatted::new(
        Value::Float(0.01),
        "1%".to_string(),
    )));
    let args = vec![arg(&catalog, "v", value)];

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "MATCH"
    );
}

#[test]
fn formatted_numeric_reannotation_preserves_source_and_exactness() {
    for (source, expected) in [
        (
            ".local $n = {42 :number} .local $p = {$n :percent} {{{$p} {$p :number}}}",
            "4,200% 42",
        ),
        (
            ".local $n = {9007199254740993 :number} .local $p = {$n :percent} {{{$p} {$p :percent}}}",
            "900,719,925,474,099,300% 900,719,925,474,099,300%",
        ),
        (
            ".local $n = {9007199254740993 :number} .local $c = {$n :currency currency=USD} {{{$c} {$c :currency currency=USD}}}",
            "$9,007,199,254,740,993.00 $9,007,199,254,740,993.00",
        ),
    ] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");
        let message = formatter.resolve("main").expect("message");
        let mut output = String::new();
        let mut diagnostics = Vec::new();

        formatter
            .format_to(message, &[], &mut output, Some(&mut diagnostics))
            .expect("formatted");

        assert_eq!(output, expected, "source={source}");
        assert!(diagnostics.is_empty(), "source={source}: {diagnostics:?}");
    }
}

#[test]
fn structured_numeric_selector_preserves_dynamic_select_diagnostics() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("main")
                .part(Part::Bind {
                    slot: 0,
                    fallback: String::from("{$n}"),
                    value: Box::new(Part::call(CallExpr::new(
                        Operand::number_literal("1"),
                        FunctionSpec::new("number"),
                    ))),
                })
                .select(
                    SelectExpr::builder(SelectorExpr::call(
                        Operand::Local(0),
                        FunctionSpec::new("number").option_var("select", "mode"),
                    ))
                    .arm("one", vec![Part::text("ONE")])
                    .default(vec![Part::text("OTHER")])
                    .build(),
                )
                .build(),
        )
        .expect("message");
    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![arg(&catalog, "mode", Value::Str("bogus".to_string()))];
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "OTHER");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::Function(MessageFunctionError::BadOption),
            crate::runtime::FormatError::BadSelector { source: None },
        ]
    );
}

#[test]
fn reannotated_failed_selector_preserves_fallback_provenance() {
    let source = ".input {$x :test:select decimalPlaces=9} \
                  .local $y = {$x :test:select decimalPlaces=1} \
                  .match $y 1.0 {{1.0}} 1 {{1}} * {{bad-option-value}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "und".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "bad-option-value");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::Function(MessageFunctionError::BadOption),
            crate::runtime::FormatError::Function(MessageFunctionError::BadOperand),
            crate::runtime::FormatError::BadSelector { source: None },
        ]
    );
}

#[test]
fn string_selector_with_options_does_not_lower_to_select_arg() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("main")
                .select(
                    SelectExpr::builder(SelectorExpr::call(
                        Operand::var("kind"),
                        FunctionSpec::new("string").option_literal("u:dir", "ltr"),
                    ))
                    .arm("formal", vec![Part::text("Good evening")])
                    .default(vec![Part::text("Hi")])
                    .build(),
                )
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_ne!(catalog.code()[0], schema::Opcode::SelectArg as u8);
    assert!(catalog.code().contains(&(schema::Opcode::CallSelect as u8)));
}

#[test]
fn markup_option_variables_are_canonicalized_like_other_variables() {
    let source = "main = {#link href=$na\u{308}me}{/link}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert!(catalog.string_id("näme").is_some());

    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let args = vec![arg(
        &catalog,
        "näme",
        Value::Str("https://example.test".to_string()),
    )];
    let mut sink = MarkupOptionSink::default();
    let mut diagnostics = vec![];
    formatter
        .format_to(message, &args, &mut sink, Some(&mut diagnostics))
        .expect("format");

    assert!(diagnostics.is_empty());
    assert_eq!(
        sink.options,
        vec![(String::from("href"), String::from("https://example.test"))]
    );
}

#[test]
fn exact_match_beats_default_even_if_default_appears_first() {
    let bytes =
        compile_str(".input { $kind :string }\n.match $kind\n* {{Hi}}\nformal {{Good evening}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");

    let out = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("formal".to_string()))],
        )
        .expect("formatted");

    assert_eq!(out, "Good evening");
}

#[test]
fn function_catalog_keys_do_not_collide_on_spaces_or_equals() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("one")
                .call(CallExpr::new(
                    Operand::literal("x"),
                    FunctionSpec::new("custom:format").option_literal("a", "b c=d"),
                ))
                .build(),
        )
        .expect("message");
    builder
        .add_message(
            Message::builder("two")
                .call(CallExpr::new(
                    Operand::literal("x"),
                    FunctionSpec::new("custom:format")
                        .option_literal("a", "b")
                        .option_literal("c", "d"),
                ))
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(catalog.func_count(), 2);
}

#[test]
fn compile_inputs_merges_multiple_files_and_tracks_origins() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "hello",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "bye",
                source: "Bye",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    assert_eq!(compiled.source_map.sources.len(), 2);
    assert_eq!(compiled.source_map.messages.len(), 2);
    assert_eq!(compiled.source_map.messages[0].message_id, "hello");
    assert_eq!(
        compiled.source_map.messages[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.line),
        Some(1)
    );

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("bye", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Bye"
    );
}

#[test]
fn compile_inputs_deduplicates_literal_text_slices() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "one.mf2",
                message_id: "one",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "two.mf2",
                message_id: "two",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(
        compiled.literal_stats.deduplication,
        LiteralDeduplication::Enabled
    );
    assert_eq!(compiled.literal_stats.literal_slices, 2);
    assert_eq!(compiled.literal_stats.unique_literals, 1);
    assert_eq!(compiled.literal_stats.duplicate_literals, 1);
    assert_eq!(compiled.literal_stats.input_literal_bytes, 10);
    assert_eq!(compiled.literal_stats.unique_literal_bytes, 5);
    assert_eq!(compiled.literal_stats.duplicate_literal_bytes, 5);
    assert_eq!(compiled.literal_stats.emitted_literal_bytes, 5);
    assert_eq!(compiled.literal_stats.saved_literal_bytes, 5);
    assert_eq!(chunk_len(&compiled.bytes, *b"LITS"), 5);
    assert_eq!(
        catalog.code(),
        &[
            schema::Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
            schema::Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
        ]
    );
}

#[test]
fn disabled_literal_deduplication_keeps_append_only_literals_without_duplicate_tracking() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "one.mf2",
                message_id: "one",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "two.mf2",
                message_id: "two",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions {
            literal_deduplication: LiteralDeduplication::Disabled,
            ..CompileOptions::default()
        },
    ));

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(chunk_len(&compiled.bytes, *b"LITS"), 10);
    assert_eq!(compiled.literal_stats.literal_slices, 2);
    assert_eq!(compiled.literal_stats.duplicate_literals, 0);
    assert_eq!(compiled.literal_stats.input_literal_bytes, 10);
    assert_eq!(compiled.literal_stats.emitted_literal_bytes, 10);
    assert_eq!(compiled.literal_stats.saved_literal_bytes, 0);
    assert_eq!(
        catalog.code(),
        &[
            schema::Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
            schema::Opcode::OutSlice as u8,
            5,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
        ]
    );
}

#[test]
fn measure_only_literal_deduplication_counts_opportunities_without_rewriting_literals() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "one.mf2",
                message_id: "one",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "two.mf2",
                message_id: "two",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions {
            literal_deduplication: LiteralDeduplication::MeasureOnly,
            ..CompileOptions::default()
        },
    ));

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(chunk_len(&compiled.bytes, *b"LITS"), 10);
    assert_eq!(compiled.literal_stats.literal_slices, 2);
    assert_eq!(compiled.literal_stats.unique_literals, 1);
    assert_eq!(compiled.literal_stats.duplicate_literals, 1);
    assert_eq!(compiled.literal_stats.duplicate_literal_bytes, 5);
    assert_eq!(compiled.literal_stats.input_literal_bytes, 10);
    assert_eq!(compiled.literal_stats.emitted_literal_bytes, 10);
    assert_eq!(compiled.literal_stats.saved_literal_bytes, 0);
    assert_eq!(
        catalog.code(),
        &[
            schema::Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
            schema::Opcode::OutSlice as u8,
            5,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
        ]
    );
}

#[test]
fn compile_inputs_deduplicates_literal_expression_slices() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "one.mf2",
                message_id: "one",
                source: "{|Hello|}",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "two.mf2",
                message_id: "two",
                source: "{|Hello|}",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(chunk_len(&compiled.bytes, *b"LITS"), 5);
    assert_eq!(
        catalog.code(),
        &[
            schema::Opcode::OutExpr as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
            schema::Opcode::OutExpr as u8,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            schema::Opcode::Halt as u8,
        ]
    );
}

#[test]
fn compile_resources_merges_named_message_bodies() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .message("hello", "Hello")
                .message("bye", "Bye"),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(compiled.source_map.sources.len(), 1);
    assert_eq!(compiled.source_map.messages.len(), 2);
    assert_eq!(compiled.source_map.messages[0].message_id, "hello");
    assert_eq!(compiled.source_map.messages[0].origin, None);

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("hello", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Hello"
    );
}

#[test]
fn compile_resources_preserves_upstream_origin_in_source_map() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(MessageResource::new("hello", "Hello").upstream_origin(
                    DetachedSourceSpan {
                        name: String::from("src/app.rs"),
                        kind: SourceKind::Rust,
                        byte_start: Some(10),
                        byte_end: Some(18),
                        line: Some(3),
                        column: Some(7),
                    },
                )),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(
        compiled.source_map.messages[0].upstream_origin,
        Some(DetachedSourceSpan {
            name: String::from("src/app.rs"),
            kind: SourceKind::Rust,
            byte_start: Some(10),
            byte_end: Some(18),
            line: Some(3),
            column: Some(7),
        })
    );
}

#[test]
fn compile_resources_maps_container_origin_into_registered_source() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(MessageResource::new("hello", "Hello").origin(ResourceSpan {
                    byte_start: 14,
                    byte_end: 21,
                    line: Some(2),
                    column: Some(13),
                })),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(
        compiled.source_map.messages[0].origin,
        Some(SourceSpan {
            source_id: SourceId(0),
            byte_start: 14,
            byte_end: 21,
            line: Some(2),
            column: Some(13),
        })
    );
}

#[test]
fn compile_resources_render_nested_message_diagnostics() {
    let err = expect_first_error(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(
                    MessageResource::new("cart.total", "Total: { $amount :number style=currency ")
                        .origin(ResourceSpan {
                            byte_start: 24,
                            byte_end: 67,
                            line: Some(6),
                            column: Some(9),
                        }),
                ),
        ],
        CompileOptions::default(),
    ));

    let rendered = err.render();
    assert!(rendered.contains("app.toml:6:9: invalid expression in message \"cart.total\""));
    assert!(rendered.contains("message \"cart.total\":1:41"));
    assert!(rendered.contains("expected matching '}'"));
    assert!(rendered.contains("found unclosed '{'"));
}

#[test]
fn compile_str_rejects_invalid_builtin_number_style_literal() {
    let err = compile_str("{ $amount :number style=curren }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "style");
            assert_eq!(expected, "\"percent\"");
            assert_eq!(found, "curren");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_builtin_number_sign_display_literal() {
    let err = compile_str("{ $amount :number signDisplay=bogus }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "signDisplay");
            assert_eq!(expected, "\"auto\", \"always\", or \"never\"");
            assert_eq!(found, "bogus");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_builtin_number_use_grouping_literal() {
    let err = compile_str("{ $amount :number useGrouping=bogus }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "useGrouping");
            assert_eq!(expected, "\"auto\", \"always\", \"never\", or \"min2\"");
            assert_eq!(found, "bogus");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_builtin_date_length_literal() {
    let err = compile_str("{|2024-01-01| :date length=full}").expect_err("must fail");

    assert!(matches!(
        err,
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } if function == "date"
            && option == "length"
            && expected == "\"short\", \"medium\", or \"long\""
            && found == "full"
    ));
}

#[test]
fn compile_str_rejects_invalid_builtin_time_precision_literal() {
    let err =
        compile_str("{|2024-01-01T12:00:00| :time precision=millisecond}").expect_err("must fail");

    assert!(matches!(
        err,
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } if function == "time"
            && option == "precision"
            && expected == "\"hour\", \"minute\", or \"second\""
            && found == "millisecond"
    ));
}

#[test]
fn compile_str_rejects_invalid_builtin_currency_display_literal() {
    let err =
        compile_str("{42 :currency currency=EUR currencyDisplay=short}").expect_err("must fail");

    assert!(matches!(
        err,
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            found,
            ..
        } if function == "currency" && option == "currencyDisplay" && found == "short"
    ));
}

#[test]
fn compile_str_rejects_invalid_builtin_option_in_nested_local_call() {
    let err =
        compile_str(".local $n = {1 :number signDisplay=bogus} .local $m = {$n :number} {{{$m}}}")
            .expect_err("must fail");
    assert!(matches!(
        err,
        CompileError::InvalidBuiltinOptionValue { function, option, .. }
            if function == "number" && option == "signDisplay"
    ));
}

#[test]
fn compile_str_rejects_invalid_plural_variant_key() {
    let err =
        compile_str(".input { $x :number select=plural }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
            .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_exact_variant_key() {
    let err =
        compile_str(".input { $x :number select=exact }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
            .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_ordinal_variant_key() {
    let err = compile_str(
        ".input { $x :number select=ordinal }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}",
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_default_number_variant_key() {
    let err = compile_str(".input { $x :number }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
        .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_accepts_number_literal_variant_key_for_plural() {
    compile_str(".input { $x :number select=plural }\n.match $x\n1 {{ONE}}\n* {{OTHER}}")
        .expect("compiled");
}

#[test]
fn resource_backed_builtin_option_error_does_not_duplicate_inner_context() {
    let err = expect_first_error(compile_resources(
        [ResourceInput::new(
            "apps/message-format-lsp/test.mf2.json",
            SourceKind::Other(String::from("json-flat")),
        )
        .resource(
            MessageResource::new("price.display", "{ $amount :number style=currenc }").origin(
                ResourceSpan {
                    byte_start: 120,
                    byte_end: 155,
                    line: Some(5),
                    column: Some(20),
                },
            ),
        )],
        CompileOptions::default(),
    ));

    let rendered = err.render();
    assert!(rendered.contains(
        "apps/message-format-lsp/test.mf2.json:5:20: invalid builtin option value in message \"price.display\""
    ));
    assert!(rendered.contains("error: invalid builtin option value"));
    assert!(rendered.contains("{ $amount :number style=currenc }"));
    assert!(rendered.contains("expected \"percent\", found currenc"));
    assert!(!rendered.contains("at source#"));
}

#[test]
fn compile_resources_collects_multiple_message_diagnostics() {
    let errs = expect_errors(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .message("bad.style", "{ $amount :number style=curren }")
                .message("bad.expr", "Total: { $amount :number style=currency "),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(errs.len(), 2);
    let rendered = CompileReport::failure(errs).render();
    assert!(rendered.contains("message \"bad.style\""));
    assert!(rendered.contains("message \"bad.expr\""));
    assert!(rendered.contains("curren"));
    assert!(rendered.contains("expected matching '}'"));
}

#[test]
fn compile_inputs_collects_multiple_input_diagnostics() {
    let errs = expect_errors(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "bad.style",
                source: "{ $amount :number style=curren }",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "bad.expr",
                source: "{ $value :number style=currency ",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    assert_eq!(errs.len(), 2);
    let rendered = CompileReport::failure(errs).render();
    assert!(rendered.contains("a.mf2"));
    assert!(rendered.contains("b.mf2"));
    assert!(rendered.contains("curren"));
    assert!(rendered.contains("unclosed '{'"));
}

#[test]
fn compile_resources_rejects_duplicate_message_ids() {
    let err = expect_first_error(compile_resources(
        [
            ResourceInput::new("a.toml", SourceKind::Other(String::from("resource-toml")))
                .message("main", "Hello"),
            ResourceInput::new("b.toml", SourceKind::Other(String::from("resource-toml")))
                .message("main", "Bye"),
        ],
        CompileOptions::default(),
    ));

    assert!(matches!(
        *err.error,
        CompileError::DuplicateMessageId { ref message_id, .. } if message_id == "main"
    ));
}

#[test]
fn catalog_builder_accepts_structured_messages_without_text_roundtrip() {
    let mut builder = CatalogBuilder::new();
    let source_id = builder
        .register_source("generated", SourceKind::Generated)
        .expect("source id");
    builder
        .add_message(
            Message::builder("hello")
                .origin(SourceSpan {
                    source_id,
                    byte_start: 10,
                    byte_end: 20,
                    line: Some(3),
                    column: Some(7),
                })
                .text("Hello from builder")
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    assert_eq!(compiled.source_map.sources[0].name, "generated");
    assert_eq!(
        compiled.source_map.messages[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.line),
        Some(3)
    );

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("hello", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Hello from builder"
    );
}

#[test]
fn compile_inputs_rejects_duplicate_message_ids() {
    let err = expect_first_error(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "main",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "main",
                source: "Bye",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    match *err.error {
        CompileError::DuplicateMessageId {
            message_id,
            first_origin,
            second_origin,
        } => {
            assert_eq!(message_id, "main");
            assert_eq!(first_origin.and_then(|origin| origin.line), Some(1));
            assert_eq!(second_origin.and_then(|origin| origin.line), Some(1));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_function() {
    let manifest = FunctionManifest::new();
    let err = compile_with_manifest(
        "main = { $value :custom:unknown }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunction {
            function,
            message_id,
            origin,
            ..
        } => {
            assert_eq!(function, "custom:unknown");
            assert_eq!(message_id.as_deref(), Some("main"));
            assert_eq!(origin.as_deref().and_then(|origin| origin.line), Some(1));
            assert!(
                origin
                    .as_deref()
                    .and_then(|origin| origin.column)
                    .is_some_and(|column| column > 1)
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_markup_name() {
    let manifest = FunctionManifest::new();
    let err = compile_with_manifest(
        "main = {#custom:link href=|/|}{/custom:link}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunction {
            function,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:link");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:format").allow_format());

    let err = compile_with_manifest(
        "main = { $value :custom:format mode=fast }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:format");
            assert_eq!(option, "mode");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_markup_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:link").allow_format());

    let err = compile_with_manifest(
        "main = {#custom:link mode=|fast|}{/custom:link}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:link");
            assert_eq!(option, "mode");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_wrong_option_value_shape() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:format")
            .allow_format()
            .option("mode", FunctionOptionValueKind::Literal),
    );

    let err = compile_with_manifest(
        "main = { $value :custom:format mode=$kind }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOptionValue {
            function,
            option,
            expected,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:format");
            assert_eq!(option, "mode");
            assert_eq!(expected, FunctionOptionValueKind::Literal);
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_missing_required_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .required_option("style", FunctionOptionValueKind::Literal),
    );

    let err = compile_with_manifest(
        "main = { $value :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::MissingFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(option, "style");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_bad_literal_number_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    let err = compile_with_manifest(
        "main = { |horse| :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(expected, FunctionOperandKind::Number);
            assert!(found.contains("horse"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_allows_variable_operand_for_number_kind() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    compile_with_manifest(
        "main = { $value :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_rejects_numeric_literal_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    let err = compile_with_manifest(
        "main = { 123 :custom:string }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:string");
            assert_eq!(expected, FunctionOperandKind::String);
            assert!(found.contains("123"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_allows_quoted_numeric_literal_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    compile_with_manifest(
        "main = { |123| :custom:string }",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_allows_quoted_numeric_local_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    compile_with_manifest(
        ".local $x = {|123|}\n{{ { $x :custom:string } }}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_rejects_quoted_numeric_literal_for_number_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    let err = compile_with_manifest(
        "main = { |123| :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(expected, FunctionOperandKind::Number);
            assert!(found.contains("123"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_selector_only_usage_in_format_position() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:select").allow_select());

    let err = compile_with_manifest(
        "main = { $value :custom:select }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnsupportedFunctionUsage {
            function,
            usage,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:select");
            assert_eq!(usage, "format");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_accepts_formatting_only_declarations() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:format").allow_format());

    for source in [
        ".local $x = {42 :custom:format} {{{$x}}}",
        ".input {$x :custom:format} {{{$x}}}",
    ] {
        compile_with_manifest(source, CompileOptions::default(), &manifest)
            .unwrap_or_else(|error| panic!("source={source}: {error}"));
    }
}

#[test]
fn compile_with_manifest_accepts_selection_only_declarations_and_aliases() {
    let manifest = custom_select_manifest(custom_select_schema());

    for source in [
        ".local $x = {a :custom:select} .match $x a {{A}} * {{OTHER}}",
        ".local $x = {a :custom:select} .local $y = {$x} .match $y a {{A}} * {{OTHER}}",
    ] {
        compile_with_manifest(source, CompileOptions::default(), &manifest)
            .unwrap_or_else(|error| panic!("source={source}: {error}"));
    }
}

#[test]
fn compile_with_manifest_requires_format_permission_for_mixed_declaration_use() {
    let manifest = custom_select_manifest(custom_select_schema());
    let err = compile_with_manifest(
        ".input {$x :custom:select} .match $x a {{{$x}}} * {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("formatting a selector-only declaration must fail");

    assert!(matches!(
        err,
        CompileError::UnsupportedFunctionUsage { usage, .. } if usage == "format"
    ));
}

#[test]
fn compile_with_manifest_propagates_selection_through_structured_aliases() {
    let selector = FunctionSpec::new("custom:select");
    let mut builder = CatalogBuilder::new();
    builder.set_function_manifest(custom_select_manifest(custom_select_schema()));
    builder
        .add_message(
            Message::builder("main")
                .part(Part::Bind {
                    slot: 0,
                    fallback: "{$x}".to_string(),
                    value: Box::new(Part::call(CallExpr::new(
                        Operand::literal("a"),
                        selector.clone(),
                    ))),
                })
                .part(Part::Bind {
                    slot: 1,
                    fallback: "{$alias}".to_string(),
                    value: Box::new(Part::Local(0)),
                })
                .select(
                    SelectExpr::builder(SelectorExpr::CheckedLocal {
                        slot: 1,
                        func: Some(selector),
                    })
                    .arm("a", vec![Part::text("A")])
                    .default(vec![Part::text("OTHER")])
                    .build(),
                )
                .build(),
        )
        .expect("message");

    expect_compiled(builder.compile());
}

#[test]
fn compile_with_manifest_checks_both_uses_without_selector_metadata() {
    let function = FunctionSpec::new("custom:format");
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:format").allow_format());
    let mut builder = CatalogBuilder::new();
    builder.set_function_manifest(manifest);
    builder
        .add_message(
            Message::builder("main")
                .part(Part::Bind {
                    slot: 0,
                    fallback: "{$x}".to_string(),
                    value: Box::new(Part::call(CallExpr::new(Operand::literal("a"), function))),
                })
                .part(Part::Local(0))
                .select(
                    SelectExpr::builder(SelectorExpr::CheckedLocal {
                        slot: 0,
                        func: None,
                    })
                    .arm("a", vec![Part::text("A")])
                    .default(vec![Part::text("OTHER")])
                    .build(),
                )
                .build(),
        )
        .expect("message");

    assert!(
        builder.compile().into_result().is_err(),
        "selection must require selection permission even without selector metadata"
    );
}

#[test]
fn compile_with_manifest_propagates_aliases_inside_select_arms() {
    let selector = FunctionSpec::new("custom:select");
    let inner = Part::Select(
        SelectExpr::builder(SelectorExpr::CheckedLocal {
            slot: 1,
            func: Some(selector.clone()),
        })
        .arm("a", vec![Part::text("A")])
        .default(vec![Part::text("OTHER")])
        .build(),
    );
    let outer = SelectExpr::builder(SelectorExpr::literal("outer"))
        .arm(
            "outer",
            vec![
                Part::Bind {
                    slot: 1,
                    fallback: "{$alias}".to_string(),
                    value: Box::new(Part::Local(0)),
                },
                inner,
            ],
        )
        .default(vec![Part::text("OTHER")])
        .build();
    let mut builder = CatalogBuilder::new();
    builder.set_function_manifest(custom_select_manifest(custom_select_schema()));
    builder
        .add_message(
            Message::builder("main")
                .part(Part::Bind {
                    slot: 0,
                    fallback: "{$x}".to_string(),
                    value: Box::new(Part::call(CallExpr::new(Operand::literal("a"), selector))),
                })
                .select(outer)
                .build(),
        )
        .expect("message");

    expect_compiled(builder.compile());
}

fn custom_select_manifest(schema: FunctionSchema) -> FunctionManifest {
    let mut manifest = FunctionManifest::new();
    manifest.insert(schema);
    manifest
}

fn custom_select_schema() -> FunctionSchema {
    FunctionSchema::new("custom:select").allow_select()
}

#[test]
fn compile_with_manifest_accepts_selector_annotation_when_allowed() {
    let manifest = custom_select_manifest(custom_select_schema());

    for source in [
        ".input { $kind :custom:select }\n.match $kind\na {{A}}\n* {{OTHER}}",
        ".input { $kind :custom:select }\n.local $alias = {$kind}\n\
         .match $alias\na {{A}}\n* {{OTHER}}",
    ] {
        compile_with_manifest(source, CompileOptions::default(), &manifest)
            .unwrap_or_else(|error| panic!("source={source}: {error}"));
    }
}

#[test]
fn compile_with_manifest_rejects_selector_key_outside_declared_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::Number),
    );

    let err = compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\nhorse {{HORSE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_selector_key_outside_plural_category_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::PluralCategory),
    );

    let err = compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\n1 {{ONE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category"));
            assert_eq!(found.as_deref(), Some("1"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_accepts_plural_category_selector_key() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::PluralCategory),
    );

    compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\none {{ONE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_accepts_selector_key_with_declared_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::Identifier),
    );

    compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\nhorse {{HORSE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_inputs_with_manifest_reports_precise_source() {
    let manifest = FunctionManifest::new();
    let err = expect_first_error(compile_inputs_with_manifest(
        [
            CompileInput {
                name: "ok.mf2",
                message_id: "first",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "bad.mf2",
                message_id: "second",
                source: "{ $value :custom:unknown }",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
        &manifest,
    ));

    assert_eq!(
        err.context
            .as_ref()
            .map(|context| context.source.name.as_str()),
        Some("bad.mf2")
    );
    match *err.error {
        CompileError::UnknownFunction { origin, .. } => {
            assert_eq!(origin.as_deref().and_then(|origin| origin.line), Some(1));
            assert!(
                origin
                    .as_deref()
                    .and_then(|origin| origin.column)
                    .is_some_and(|column| column > 1)
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_inputs_with_manifest_reports_declaration_function_origin() {
    let source = ".input { $value :custom:unknown }\n{{Hello}}";
    let err = expect_first_error(compile_inputs_with_manifest(
        [CompileInput {
            name: "bad.mf2",
            message_id: "main",
            source,
            kind: SourceKind::MessageFormat,
        }],
        CompileOptions::default(),
        &FunctionManifest::new(),
    ));

    match *err.error {
        CompileError::UnknownFunction { origin, .. } => {
            let origin = origin.expect("function origin");
            assert_eq!(origin.byte_start, source.find(":custom").expect("function"));
            assert_eq!(origin.byte_end, source.find(" }").expect("function end"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn render_with_source_highlights_unknown_function_annotation() {
    let err = compile_with_manifest(
        "main = { $value :custom:unknown }",
        CompileOptions::default(),
        &FunctionManifest::new(),
    )
    .expect_err("must fail");

    let rendered = err.render_with_source("main = { $value :custom:unknown }");
    assert!(rendered.contains("error: unknown function"));
    assert!(rendered.contains("--> <input>:1:"));
    assert!(rendered.contains("custom:unknown"));
}

#[test]
fn function_fallback_reannotation_reports_cascading_bad_operand() {
    let source = ".local $var = {|val| :test:undefined} {{{$var :test:function}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &[], &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "{$var}");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::UnknownFunction { fn_id: 0 },
            crate::runtime::FormatError::Function(MessageFunctionError::BadOperand),
        ]
    );
}

#[test]
fn universal_id_is_retained_for_custom_host_results() {
    let bytes = compile_str("{foo :custom u:id=first}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_fn_id, _args, _opts| Ok(Value::Str("result".to_string())));
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut sink = UniversalIdSink::default();
    formatter
        .format_to(message, &[], &mut sink, None)
        .expect("formatted");
    assert_eq!(sink.output, "result");
    assert_eq!(sink.ids, ["first"]);
}

#[test]
fn delegated_builtin_values_preserve_structured_metadata_and_text() {
    struct CallOnlyBuiltinHost(BuiltinHost);

    impl Host for CallOnlyBuiltinHost {
        type CatalogIndex = crate::runtime::BuiltinHostCatalogIndex;

        fn index(
            &mut self,
            catalog: &Catalog,
        ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
            self.0.index(catalog)
        }

        fn call(
            &mut self,
            catalog: &Catalog,
            index: &Self::CatalogIndex,
            fn_id: u16,
            args: &[Value],
            opts: FunctionOptions<'_>,
            on_error: &mut dyn FnMut(MessageFunctionError),
        ) -> Result<Value, HostCallError> {
            self.0.call(catalog, index, fn_id, args, opts, on_error)
        }
    }

    let bytes = compile_str("{42 :number u:id=first}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = CallOnlyBuiltinHost(BuiltinHost::new(&locale).expect("host"));
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut sink = UniversalIdSink::default();

    formatter
        .format_to(message, &[], &mut sink, None)
        .expect("formatted");

    assert_eq!(sink.output, "42");
    assert_eq!(sink.ids, ["first"]);

    let bytes = compile_str("{world :string u:dir=rtl u:id=first}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = CallOnlyBuiltinHost(BuiltinHost::new(&locale).expect("host"));
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut plain = String::new();
    formatter
        .format_to(message, &[], &mut plain, None)
        .expect("formatted");
    let mut structured = UniversalIdSink::default();
    formatter
        .format_to(message, &[], &mut structured, None)
        .expect("formatted");

    assert_eq!(plain, "\u{2067}world\u{2069}");
    assert_eq!(structured.output, plain);
    assert_eq!(structured.ids, ["first"]);
}

#[test]
fn universal_id_does_not_change_custom_host_selection() {
    let source = ".local $x={foo :custom u:id=first} .match $x a {{MATCH}} * {{OTHER}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_fn_id, args: &[Value], _opts| match args.first() {
        Some(Value::StrRef(_)) => Ok(Value::Str("a".to_string())),
        Some(Value::Str(value)) => Ok(Value::Str(value.clone())),
        _ => Err(HostCallError::Function(MessageFunctionError::BadOperand)),
    });
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");

    for structured in [false, true] {
        let mut diagnostics = Vec::new();
        if structured {
            let mut sink = UniversalIdSink::default();
            formatter
                .format_to(message, &[], &mut sink, Some(&mut diagnostics))
                .expect("formatted");
            assert_eq!(sink.output, "MATCH");
        } else {
            let mut output = String::new();
            formatter
                .format_to(message, &[], &mut output, Some(&mut diagnostics))
                .expect("formatted");
            assert_eq!(output, "MATCH");
        }
        assert!(diagnostics.is_empty());
    }
}

#[test]
fn optionless_string_reannotation_retains_inherited_universal_id() {
    let source = ".local $x={foo :custom u:id=first} {{{$x :string}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_fn_id, _args, _opts| Ok(Value::Str("result".to_string())));
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut sink = UniversalIdSink::default();

    formatter
        .format_to(message, &[], &mut sink, None)
        .expect("formatted");

    assert_eq!(sink.output, "result");
    assert_eq!(sink.ids, ["first"]);
}

#[test]
fn explicit_universal_id_overrides_inherited_id() {
    let source = ".local $x={foo :custom u:id=first} {{{$x :custom u:id=second}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_fn_id, args: &[Value], _opts| {
        Ok(args
            .first()
            .cloned()
            .unwrap_or_else(|| Value::Str("result".to_string())))
    });
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut sink = UniversalIdSink::default();

    formatter
        .format_to(message, &[], &mut sink, None)
        .expect("formatted");

    assert_eq!(sink.output, "foo");
    assert_eq!(sink.ids, ["second"]);
}

#[test]
fn universal_id_does_not_change_dynamic_option_values_seen_by_custom_hosts() {
    let source = ".local $o={a :custom u:id=opt} {{{a :custom option=$o}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_fn_id, _args: &[Value], opts: FunctionOptions<'_>| {
        let options = opts.iter().collect::<Vec<_>>();
        if options.is_empty() {
            return Ok(Value::Str("a".to_string()));
        }
        if matches!(options.first(), Some((_, Value::Str(_)))) {
            Ok(Value::Str("OK".to_string()))
        } else {
            Err(HostCallError::Function(MessageFunctionError::BadOption))
        }
    });
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");

    for structured in [false, true] {
        let mut diagnostics = Vec::new();
        if structured {
            let mut sink = UniversalIdSink::default();
            formatter
                .format_to(message, &[], &mut sink, Some(&mut diagnostics))
                .expect("formatted");
            assert_eq!(sink.output, "OK");
        } else {
            let mut output = String::new();
            formatter
                .format_to(message, &[], &mut output, Some(&mut diagnostics))
                .expect("formatted");
            assert_eq!(output, "OK");
        }
        assert!(diagnostics.is_empty());
    }
}

#[test]
fn optionless_string_reannotation_reports_cascading_bad_operand() {
    let source = ".local $var = {|val| :test:undefined} {{{$var :string}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut output = String::new();
    let mut diagnostics = Vec::new();

    formatter
        .format_to(message, &[], &mut output, Some(&mut diagnostics))
        .expect("formatted");

    assert_eq!(output, "{$var}");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::UnknownFunction { fn_id: 0 },
            crate::runtime::FormatError::Function(MessageFunctionError::BadOperand),
        ]
    );
}

#[test]
fn function_options_allow_grammar_whitespace_around_equals() {
    for source in ["{:f k= v}", "{:f k = v}"] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");
        let message = formatter.resolve("main").expect("message");
        let mut output = String::new();
        let mut diagnostics = Vec::new();

        formatter
            .format_to(message, &[], &mut output, Some(&mut diagnostics))
            .expect("formatted");

        assert_eq!(output, "{:f}", "source={source}");
        assert_eq!(
            diagnostics,
            vec![crate::runtime::FormatError::UnknownFunction { fn_id: 0 }],
            "source={source}"
        );
    }
}

#[test]
fn unresolved_operand_still_reports_an_unknown_function() {
    for source in ["{$x :f}", "{$x :u:f}"] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");
        let message = formatter.resolve("main").expect("message");
        let mut output = String::new();
        let mut diagnostics = Vec::new();

        formatter
            .format_to(message, &[], &mut output, Some(&mut diagnostics))
            .expect("formatted");

        assert_eq!(output, "{$x}", "source={source}");
        assert_eq!(
            diagnostics,
            vec![
                crate::runtime::FormatError::MissingArg("x".to_string()),
                crate::runtime::FormatError::UnknownFunction { fn_id: 0 },
            ],
            "source={source}"
        );
    }
}

#[test]
fn malformed_expression_tails_remain_syntax_errors() {
    for source in [
        "missing space {42:func}",
        "missing space {|foo|:func}",
        "missing space {:func@bar}",
        "missing space {:func @bar@baz}",
        "missing space {:func @bar=42@baz}",
        "bad {:placeholder option=}",
        "bad {:placeholder:}",
        "bad {:placeholder::foo}",
        "bad {:placeholder :option=x}",
        "bad {:placeholder @attribute=}",
        "bad {:placeholder @attribute=@foo}",
        "bad {:placeholder @attribute=$foo}",
    ] {
        assert!(compile_str(source).is_err(), "source={source}");
    }
}

#[test]
fn catalog_builder_uses_manifest_for_structured_messages() {
    let mut builder = CatalogBuilder::new();
    builder.set_function_manifest(FunctionManifest::new());
    builder
        .add_message(
            Message::builder("main")
                .call(CallExpr::new(
                    Operand::var("value"),
                    FunctionSpec::new("custom:missing"),
                ))
                .build(),
        )
        .expect("message");

    let err = expect_first_error(builder.compile());
    match *err.error {
        CompileError::UnknownFunction {
            function,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:missing");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn catalog_builder_builder_api_formats_structured_select() {
    let mut builder = CatalogBuilder::new();
    let source_id = builder
        .register_source("generated", SourceKind::Generated)
        .expect("source id");
    builder
        .add_message(
            Message::builder("main")
                .origin(SourceSpan {
                    source_id,
                    byte_start: 0,
                    byte_end: 42,
                    line: Some(1),
                    column: Some(1),
                })
                .select(
                    SelectExpr::builder(SelectorExpr::var("kind"))
                        .arm("formal", vec![Part::text("Good evening")])
                        .default(vec![Part::text("Hi")])
                        .build(),
                )
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

    let formal = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("formal".to_string()))],
        )
        .expect("formatted");
    let casual = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("casual".to_string()))],
        )
        .expect("formatted");

    assert_eq!(formal, "Good evening");
    assert_eq!(casual, "Hi");
}

#[test]
fn compiles_and_formats_interpolation() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World!");
}

#[test]
fn default_bidi_isolation_rewrites_bare_interpolation_to_string_call() {
    let messages = parse_messages(
        "{ $name }",
        CompileOptions {
            default_bidi_isolation: true,
            ..CompileOptions::default()
        },
    )
    .expect("parsed");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "main");
    match messages[0].parts.as_slice() {
        [
            Part::Call(CallExpr {
                operand: Operand::Var(var),
                func,
                ..
            }),
        ] => {
            assert_eq!(var, "name");
            assert_eq!(func.name, "string");
        }
        other => panic!("unexpected lowered parts: {other:?}"),
    }
}

#[test]
fn default_bidi_isolation_rewrites_bare_literal_expression_to_string_call() {
    let messages = parse_messages(
        "{ hello }",
        CompileOptions {
            default_bidi_isolation: true,
            ..CompileOptions::default()
        },
    )
    .expect("parsed");
    assert_eq!(messages.len(), 1);
    match messages[0].parts.as_slice() {
        [
            Part::Call(CallExpr {
                operand: Operand::Literal { value, kind },
                func,
                ..
            }),
        ] => {
            assert_eq!(value, "hello");
            assert_eq!(*kind, OperandLiteralKind::String);
            assert_eq!(func.name, "string");
        }
        other => panic!("unexpected lowered parts: {other:?}"),
    }
}

#[test]
fn compiles_and_formats_call() {
    let source = "{ $n :double }";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(|_fn_id, _args, _opts| Ok(Value::Str("CALLED".to_string()))),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "n", Value::Int(12))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "CALLED"
    );
}

#[test]
fn local_declaration_is_evaluated_once_and_selected_through_the_host() {
    let bytes = compile_str(
        ".local $n = {seed :test:format} .match $n called {{hit {$n} {$n}}} * {{miss}}",
    )
    .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    struct CountingHost {
        calls: Rc<Cell<u32>>,
        selects: Rc<Cell<u32>>,
    }

    impl Host for CountingHost {
        type CatalogIndex = ();

        fn index(
            &mut self,
            _catalog: &Catalog,
        ) -> Result<Self::CatalogIndex, crate::runtime::FormatError> {
            Ok(())
        }

        fn call(
            &mut self,
            _catalog: &Catalog,
            _index: &Self::CatalogIndex,
            _fn_id: u16,
            _args: &[Value],
            _opts: FunctionOptions<'_>,
            _on_error: &mut dyn FnMut(MessageFunctionError),
        ) -> Result<Value, HostCallError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Value::Str("called".to_string()))
        }

        fn call_select(
            &mut self,
            _catalog: &Catalog,
            _index: &Self::CatalogIndex,
            _fn_id: u16,
            _args: &[Value],
            _opts: FunctionOptions<'_>,
            _on_error: &mut dyn FnMut(MessageFunctionError),
        ) -> Result<Value, HostCallError> {
            self.selects.set(self.selects.get() + 1);
            Ok(Value::Str("called".to_string()))
        }
    }

    let calls = Rc::new(Cell::new(0));
    let selects = Rc::new(Cell::new(0));
    let mut formatter = Formatter::new(
        &catalog,
        CountingHost {
            calls: Rc::clone(&calls),
            selects: Rc::clone(&selects),
        },
    )
    .expect("formatter");
    let output = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(output, "hit called called");
    assert_eq!(calls.get(), 1);
    assert_eq!(selects.get(), 1);
}

#[test]
fn invalid_expr_fails() {
    let err = compile_str("{}").expect_err("must fail");
    assert!(matches!(err, CompileError::InvalidExpr { .. }));
}

#[test]
fn reversed_quoted_pattern_delimiters_return_errors_without_panicking() {
    for source in ["}}{{", ".local $x = {1} }}{{"] {
        assert!(compile_str(source).is_err(), "source={source}");
    }
}

#[test]
fn lower_expression_node_requires_typed_non_select_payload() {
    let expr = crate::compiler::syntax::ast::ExpressionNode {
        raw_span: 1..6,
        span: 1..6,
        kind: crate::compiler::syntax::ast::ExpressionKindNode::Var,

        payload: None,
        diag_hint: None,
    };
    let err = lower_expression_node_to_part(
        "{ $x }",
        &expr,
        crate::compiler::syntax::span::SourceContext::Line {
            line: 1,
            column_offset: 0,
        },
        None,
    )
    .expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("typed expression payload"));
            assert_eq!(found.as_deref(), Some("missing expression payload"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_selector_reports_expected_and_found() {
    let err = compile_str(".match").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            line,
            column,
            expected,
            found,
        } => {
            assert_eq!(line, 1);
            assert_eq!(expected, Some("at least one selector variable"));
            assert_eq!(found.as_deref(), Some("none"));
            assert!(column >= 1);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn duplicate_variant_key_reports_collision_context() {
    let err = compile_str(".input {$x :string} .match $x Ḍ̇ {{a}} Ḍ̇ {{b}} * {{c}}")
        .expect_err("must fail");
    match err {
        CompileError::DuplicateVariant {
            expected,
            found,
            line,
            column,
        } => {
            assert_eq!(line, 1);
            assert!(column >= 1);
            assert_eq!(expected, Some("unique variant key (NFC-normalized)"));
            let found = found.expect("found");
            assert!(found.contains("duplicates key first seen at 1:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_function_name_after_colon_reports_expected_and_found() {
    let err = compile_str("{ $n : }").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("function name after ':'"));
            assert_eq!(found.as_deref(), Some("end of expression"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn malformed_option_token_reports_expected_and_found() {
    let err = compile_str("{ $n :number minimumFractionDigits }").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(
                expected,
                Some("option token 'key=value' or attribute '@name'")
            );
            assert!(
                found
                    .as_deref()
                    .is_some_and(|value| value.contains("minimumFractionDigits"))
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn invalid_var_reports_column_and_display() {
    let err = compile_str("{ $-bad }").expect_err("must fail");
    match &err {
        CompileError::InvalidVar {
            line,
            column,
            expected,
            found,
        } => {
            assert_eq!(*line, 1);
            assert_eq!(*column, 4);
            assert_eq!(*expected, Some("valid identifier after '$'"));
            assert!(found.as_ref().is_some_and(|it| it.contains("$-bad")));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let message = err.to_string();
    assert!(message.contains("1:4"));
    assert!(message.contains("expected valid identifier after '$'"));
}

#[test]
fn renders_diagnostic_with_source_snippet() {
    let source = "{ $-bad }";
    let err = compile_str(source).expect_err("must fail");
    let rendered = err.render_with_source(source);
    assert!(rendered.contains("error: invalid variable"));
    assert!(rendered.contains("--> <input>:1:4"));
    assert!(rendered.contains("1 | { $-bad }"));
    assert!(rendered.contains("^ expected valid identifier after '$'"));
}

#[test]
fn renders_diagnostic_with_custom_label() {
    let source = ".match";
    let err = compile_str(source).expect_err("must fail");
    let rendered = err.render_with_source_label(source, "demo.mf2");
    assert!(rendered.contains("--> demo.mf2:1:"));
}

#[test]
fn render_with_source_falls_back_for_non_context_errors() {
    let err = CompileError::TooManyStrings;
    let rendered = err.render_with_source("hello = world");
    assert_eq!(rendered, "too many interned strings");
}

#[test]
fn render_with_source_falls_back_when_line_is_out_of_range() {
    let err = CompileError::InvalidExpr {
        line: 5,
        column: 2,
        expected: Some("expression"),
        found: Some(String::from("eof")),
    };
    let rendered = err.render_with_source("one line");
    assert_eq!(
        rendered,
        "invalid expression at 5:2 (expected expression, found eof)"
    );
}

#[test]
fn render_with_source_handles_zero_column_without_panic() {
    let err = CompileError::InvalidExpr {
        line: 1,
        column: 0,
        expected: Some("expression"),
        found: Some(String::from("eof")),
    };
    let rendered = err.render_with_source("hello");
    assert!(rendered.contains("--> <input>:1:0"));
    assert!(rendered.contains("| ^ expected expression, found eof"));
}

#[test]
fn render_with_source_targets_specific_multiline_row() {
    let err = CompileError::MissingDefaultArm {
        line: 3,
        column: 1,
        expected: Some("default arm '*[key]...'"),
        found: Some(String::from("none")),
    };
    let source = "a = A\nb = B\ng = .match $x a {{A}}";
    let rendered = err.render_with_source(source);
    assert!(rendered.contains("--> <input>:3:1"));
    assert!(rendered.contains("3 | g = .match $x a {{A}}"));
    assert!(rendered.contains("^ expected default arm '*[key]...', found none"));
}

#[test]
fn match_keyword_spacing_error_uses_character_column() {
    let err = compile_str(".match\u{16A0} $x * {{ok}}").expect_err("must fail");
    let rendered = err.render_with_source(".match\u{16A0} $x * {{ok}}");
    assert!(rendered.contains("error: invalid expression"));
    assert!(rendered.contains("--> <input>:1:7"));
    assert!(rendered.contains("expected whitespace after '.match'"));
}

#[test]
fn literal_expression_formats() {
    let bytes = compile_str("hello {world}{|!|}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "hello world!");
}

#[test]
fn raw_message_source_uses_main_id() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("WG".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello WG!");
}

#[test]
fn raw_quoted_pattern_is_extracted() {
    let bytes = compile_str(".input {$x} {{Hello}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Hello");
}

#[test]
fn local_literal_declaration_is_substituted() {
    let bytes = compile_str(".local $x = {1} {{A {$x} B}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "A 1 B");
}

#[test]
fn leading_declarations_without_quoted_pattern_are_stripped() {
    let bytes = compile_str(".input {$name} .local $x = {ok} {{Hello {$x}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Hello ok!");
}

#[test]
fn single_message_declarations_require_quoted_body() {
    let err = compile_str(".input {$name} Hello {$name}").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("message body '{{...}}' after declarations"));
            assert_eq!(found.as_deref(), Some("end of input"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn raw_match_statement_is_rewritten_and_formatted() {
    let source =
        ".input {$kind :string} .match $kind formal {{Good evening}} casual {{Hi}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");

    let args1 = vec![arg(&catalog, "kind", Value::Str("formal".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args1)
            .expect("formatted"),
        "Good evening"
    );

    let args2 = vec![arg(&catalog, "kind", Value::Str("casual".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args2)
            .expect("formatted"),
        "Hi"
    );

    let args3 = vec![arg(&catalog, "kind", Value::Str("other".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args3)
            .expect("formatted"),
        "Hello"
    );
}

#[test]
fn raw_match_with_local_literal_string_resolves_through_host() {
    let source = ".local $kind = {formal :string} .match $kind formal {{Good evening}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Good evening");
}

#[test]
fn raw_match_with_local_alias_selector_uses_input() {
    let source = ".input {$kind :string} .local $k = {$kind} .local $k2 = {$k} \
                  .match $k2 formal {{Good evening}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");
    let args = vec![arg(&catalog, "kind", Value::Str("formal".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Good evening");
}

#[test]
fn raw_match_rejects_unannotated_local_alias_selector() {
    let error = compile_str(
        ".input {$seed} .local $alias = {$seed} .local $alias2 = {$alias} \
         .match $alias2 a {{A}} * {{OTHER}}",
    )
    .expect_err("an eager alias slot does not supply a selector annotation");

    assert!(matches!(
        error,
        CompileError::MissingSelectorAnnotation { .. }
    ));
}

#[test]
fn raw_match_with_numeric_local_selector_falls_back_to_plural_category() {
    for arms in [
        "1 a {{exact-a}} one b {{plural-b}} * * {{fallback}}",
        "* * {{fallback}} one b {{plural-b}} 1 a {{exact-a}}",
        "* * {{fallback}} 1 a {{exact-a}} one b {{plural-b}}",
    ] {
        let source =
            alloc::format!(".input {{$n :number}} .input {{$s :string}} .match $n $s {arms}");
        let bytes = compile_str(&source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");
        let args = vec![
            arg(&catalog, "n", Value::Int(1)),
            arg(&catalog, "s", Value::Str("b".to_string())),
        ];
        assert_eq!(
            formatter
                .format_by_id_for_test("main", &args)
                .expect("formatted"),
            "plural-b"
        );

        let exact_args = vec![
            arg(&catalog, "n", Value::Int(1)),
            arg(&catalog, "s", Value::Str("a".to_string())),
        ];
        assert_eq!(
            formatter
                .format_by_id_for_test("main", &exact_args)
                .expect("formatted"),
            "exact-a"
        );
    }
}

#[test]
fn raw_match_with_percent_input_selects_the_scaled_plural_category() {
    let source = ".input {$n :percent} .match $n one {{one}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");

    for (value, expected) in [(0.01, "one"), (1.0, "other")] {
        let args = vec![arg(&catalog, "n", Value::Float(value))];
        let message = formatter.resolve("main").expect("message");
        let mut output = String::new();
        let mut diagnostics = Vec::new();
        formatter
            .format_to(message, &args, &mut output, Some(&mut diagnostics))
            .expect("formatted");
        assert_eq!(output, expected, "value={value}");
        assert!(diagnostics.is_empty(), "value={value}: {diagnostics:?}");
    }
}

#[test]
fn stored_percent_retains_scaled_exact_value_and_dynamic_precision() {
    for (source, args, expected) in [
        (
            ".local $pct = {1 :percent} .match $pct 1 {{one}} 100 {{hundred}} * {{other}}",
            Vec::new(),
            "hundred",
        ),
        (
            ".input {$n :percent minimumFractionDigits=$digits maximumFractionDigits=$digits} .match $n one {{one}} * {{other}}",
            vec![("n", Value::Float(0.01)), ("digits", Value::Int(2))],
            "other",
        ),
    ] {
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let locale = "en".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(&catalog, host).expect("formatter");
        let args = args
            .into_iter()
            .map(|(name, value)| arg(&catalog, name, value))
            .collect::<Vec<_>>();
        assert_eq!(
            formatter
                .format_by_id_for_test("main", &args)
                .expect("formatted"),
            expected
        );
    }
}

#[test]
fn stored_percent_string_reannotation_uses_formatted_presentation() {
    let bytes = compile_str(".local $pct = {0.01 :percent} {{{$pct :string}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &[])
            .expect("formatted"),
        "1%"
    );
}

#[test]
fn percent_reannotation_inherits_resolved_precision_for_output() {
    let source =
        ".local $p={0.01 :percent minimumFractionDigits=4} {{{$p}|{$p :percent}|{$p :number}}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &[])
            .expect("formatted"),
        "1.0000%|1.0000%|0.0100"
    );
}

#[test]
fn percent_reannotation_inherits_resolved_precision_for_selection() {
    let source = ".local $p={0.01 :percent minimumFractionDigits=$digits} .local $q={$p :percent} .match $q one {{one}} other {{other}} * {{fallback}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = [arg(&catalog, "digits", Value::Int(2))];

    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "other"
    );
}

#[test]
fn raw_match_rechecks_each_source_local_selector_once() {
    let source =
        ".input {$n :number} .match $n $n 1 1 {{exact}} one one {{category}} * * {{fallback}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let mut sink = String::new();
    let mut diagnostics = Vec::new();
    formatter
        .format_to(message, &[], &mut sink, Some(&mut diagnostics))
        .expect("formatted");
    assert_eq!(sink, "fallback");
    assert_eq!(
        diagnostics,
        vec![
            crate::runtime::FormatError::MissingArg("n".to_string()),
            crate::runtime::FormatError::Function(MessageFunctionError::BadOperand),
            crate::runtime::FormatError::BadSelector { source: None },
            crate::runtime::FormatError::BadSelector { source: None },
        ]
    );
}

#[test]
fn raw_match_with_offset_numeric_local_selector_falls_back_to_plural_category() {
    let source = ".local $n = {2 :number} .local $m = {$n :offset subtract=1} .input {$s :string} .match $m $s 1 a {{exact-a}} one b {{plural-b}} * * {{fallback}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![arg(&catalog, "s", Value::Str("b".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "plural-b"
    );
}

#[test]
fn local_alias_is_substituted() {
    let bytes = compile_str(".local $a = {$name} {{Hello {$a}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World!");
}

#[test]
fn chained_local_aliases_are_substituted() {
    let bytes =
        compile_str(".local $a = {$name} .local $b = {$a} {{Hello {$b}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("Chain".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello Chain!");
}

#[test]
fn long_alias_chain_is_substituted() {
    let bytes = compile_str(
        ".input {$seed} \
         .local $a = {$seed} \
         .local $b = {$a} \
         .local $c = {$b} \
         .local $d = {$c} \
         .local $e = {$d} \
         .local $f = {$e} \
         .local $g = {$f} \
         .local $h = {$g} \
         .local $i = {$h} \
         .local $j = {$i} \
         {{Hello {$j}!}}",
    )
    .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "seed", Value::Str("Chain".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello Chain!");
}

#[test]
fn local_integer_function_is_evaluated() {
    let bytes = compile_str(".local $x = {4.2 :integer} {{X={$x}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "X=4");
}

#[test]
fn local_test_select_decimal_places_is_evaluated_by_the_host() {
    let source = ".local $x = {1 :test:select decimalPlaces=1} .match $x 1.0 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let host = HostFn(|_, _: &[Value], _: FunctionOptions<'_>| Ok(Value::Str(String::from("1.0"))));
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "A");
}

#[test]
fn resolved_test_selector_preserves_exact_decimal_text() {
    let source = ".local $x = {1 :test:select decimalPlaces=1} .match $x 1 {{integer}} 1.0 {{decimal}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en-US".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "decimal");
}

#[test]
fn raw_match_with_dynamic_select_option_uses_default_arm() {
    let source =
        ".input {$mode} .local $x = {1 :test:select select=$mode} .match $x 1 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let code = opcodes(&catalog);
    assert!(code.contains(&schema::Opcode::StoreLocal));
    assert!(code.contains(&schema::Opcode::CheckSelector));
    assert!(code.contains(&schema::Opcode::ProjectSelect));
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "B");
}

#[test]
fn raw_match_with_unstable_selector_chain_uses_default_arm() {
    let source = ".input {$mode} .local $a = {1 :test:select select=$mode} .local $x = {$a :integer} .match $x 1 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "B");
}

#[test]
fn raw_match_with_two_local_selectors_resolves_through_the_host() {
    let source = ".local $x = {1 :test:select} .local $y = {0 :test:select} .match $x $y 1 1 {{1,1}} 1 * {{1,*}} * 1 {{*,1}} * * {{*,*}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "1,*");
}

#[test]
fn raw_match_with_two_runtime_selectors_is_rewritten() {
    let source = ".input {$x :string} .input {$y :string} .match $x $y 1 1 {{1,1}} * * {{*,*}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");
    let args_11 = vec![
        arg(&catalog, "x", Value::Int(1)),
        arg(&catalog, "y", Value::Int(1)),
    ];
    let args_10 = vec![
        arg(&catalog, "x", Value::Int(1)),
        arg(&catalog, "y", Value::Int(0)),
    ];

    let out_11 = formatter
        .format_by_id_for_test("main", &args_11)
        .expect("formatted");
    assert_eq!(out_11, "1,1");

    let out_10 = formatter
        .format_by_id_for_test("main", &args_10)
        .expect("formatted");
    assert_eq!(out_10, "*,*");
}

#[test]
fn raw_match_with_escaped_quoted_key_selects_expected_arm() {
    let source = ".input {$kind :string} .match $kind |a\\|b| {{pipe}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, passthrough_host()).expect("formatter");
    let args_hit = vec![arg(&catalog, "kind", Value::Str(String::from("a|b")))];
    let args_default = vec![arg(&catalog, "kind", Value::Str(String::from("x")))];

    let out_hit = formatter
        .format_by_id_for_test("main", &args_hit)
        .expect("formatted");
    assert_eq!(out_hit, "pipe");

    let out_default = formatter
        .format_by_id_for_test("main", &args_default)
        .expect("formatted");
    assert_eq!(out_default, "other");
}

#[test]
fn raw_match_with_integer_select_exact_formats_literal_arm() {
    let source = ".local $sel = {1 :integer select=exact} .match $sel 1 {{literal select {$sel}}} * {{OTHER}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "literal select 1");
}

#[test]
fn local_offset_result_can_be_used_in_match_selector() {
    let source = ".local $x = {10 :integer} .local $y = {$x :offset subtract=6} .match $y 10 {{=10}} 4 {{=4}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "=4");
}

#[test]
fn raw_offset_result_defaults_to_plural_selection() {
    let source = ".local $n = {2 :offset subtract=1} .match $n one {{one}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "one");
}

#[test]
fn missing_default_match_fails() {
    let err = compile_str(".input { $x :string }\n.match $x\na {{A}}").expect_err("must fail");
    assert!(matches!(err, CompileError::MissingDefaultArm { .. }));
}

#[test]
fn rejects_private_use_annotation() {
    compile_str("{!.\\{}}").expect_err("private-use annotation must fail");
    compile_str("{!. \\{}}").expect_err("private-use annotation with space must fail");
}

#[test]
fn rejects_reserved_annotation_after_literal() {
    compile_str("hello {|4.2| %num\\{be\\|r}").expect_err("reserved annotation must fail");
}

#[test]
fn bare_plus_is_valid_literal() {
    compile_str("{+}").expect("bare + should compile");
}

// ─── Quoted literal charset (mf-yr4e) ────────────────────────────────

#[test]
fn quoted_literal_allows_braces_inside() {
    let bytes = compile_str("{|foo{bar}baz|}").expect("braces inside quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "foo{bar}baz");
}

#[test]
fn quoted_literal_allows_dot_and_at() {
    let bytes = compile_str("{|a.b@c|}").expect("dot and at inside quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "a.b@c");
}

// ─── MF2 whitespace strictness (mf-5h50) ─────────────────────────────

#[test]
fn nbsp_inside_expression_is_not_whitespace() {
    // NBSP (U+00A0) should NOT be treated as whitespace separator
    compile_str("{ $x\u{00A0}:number }").expect_err("NBSP must not act as whitespace");
}

#[test]
fn ideographic_space_is_valid_whitespace() {
    // Ideographic space (U+3000) IS valid MF2 whitespace
    compile_str("{\u{3000}$x :string }").expect("ideographic space should be valid ws");
}

// ─── Bidi-only whitespace (mf-v289) ──────────────────────────────────

#[test]
fn bidi_only_between_function_and_option_fails() {
    // Bidi-only between function and option must not satisfy mandatory ws
    compile_str("{ $x :number\u{200E}style=percent }")
        .expect_err("bidi-only must not satisfy mandatory ws");
}

#[test]
fn bidi_plus_space_between_function_and_option_succeeds() {
    compile_str("{ $x :number\u{200E} style=percent }").expect("bidi + space should satisfy ws");
}

// ─── Escape sequence validation (mf-1mha) ────────────────────────────

#[test]
fn text_escape_backslash_n_fails() {
    compile_str("hello\\nworld").expect_err("\\n is not a valid escape in text");
}

#[test]
fn text_escape_backslash_backslash_succeeds() {
    let bytes = compile_str("hello\\\\world").expect("\\\\");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "hello\\world");
}

#[test]
fn text_escape_open_brace_succeeds() {
    let bytes = compile_str("hello\\{world").expect("\\{");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "hello{world");
}

#[test]
fn quoted_escape_pipe_succeeds() {
    let bytes = compile_str("{|a\\|b|}").expect("\\| in quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "a|b");
}

#[test]
fn quoted_escape_backslash_n_fails() {
    compile_str("{|a\\nb|}").expect_err("\\n is not valid in quoted literal");
}

// ─── Bidi around identifiers (mf-s5fv) ───────────────────────────────

#[test]
fn function_with_leading_bidi() {
    compile_str("{ $x :\u{200E}number }").expect("bidi before function name");
}

#[test]
fn markup_with_leading_bidi() {
    compile_str("{#\u{200E}bold}text{/bold}").expect("bidi before markup name");
}

#[test]
fn markup_duplicate_options_are_rejected() {
    let err = compile_str("{#link href=one href=two}text{/link}").expect_err("must fail");
    assert!(matches!(err, CompileError::DuplicateOptionName { .. }));
}

#[test]
fn local_literal_option_values_are_substituted_as_semantic_text() {
    let bytes =
        compile_str(".local $mode = {|fast path|} {{Value: { $name :custom:format mode=$mode }}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert!(catalog.string_id("fast path").is_some());
    assert!(catalog.string_id("|fast path|").is_none());
}

#[test]
fn custom_host_observes_local_literal_option_provenance() {
    let bytes =
        compile_str(".local $mode = {|fast path|} {{Value: { $name :custom:format mode=$mode }}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mode_id = catalog.string_id("mode").expect("mode id");
    let fast_id = catalog.string_id("fast path").expect("fast path id");
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(move |_, _, opts| {
            assert_eq!(opts.get(mode_id), Some(&Value::StrRef(fast_id)));
            assert!(opts.was_dynamic(mode_id));
            assert!(!opts.was_unresolved(mode_id));
            Ok(Value::Str("ok".to_string()))
        }),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("name".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "Value: ok"
    );
}

#[test]
fn custom_host_observes_omitted_missing_option_without_diagnostics() {
    let bytes =
        compile_str(".local $bad = {$missing} {{Value: { $name :custom:format mode=$bad }}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mode_id = catalog.string_id("mode").expect("mode id");
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(move |_, _, opts: FunctionOptions<'_>| {
            observed.set(observed.get() + 1);
            assert!(opts.is_empty());
            assert!(opts.was_dynamic(mode_id));
            assert!(opts.was_unresolved(mode_id));
            Ok(Value::Str("ok".to_string()))
        }),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("name".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "Value: ok"
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn optionless_string_input_is_resolved_without_the_host() {
    let bytes = compile_str(".input {$x :string} {{A={$x} B={$x}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(move |_, _, _| {
            observed.set(observed.get() + 1);
            Ok(Value::Str("resolved".to_string()))
        }),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "x", Value::Int(1))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "A=1 B=1"
    );
    assert_eq!(calls.get(), 0);
}

// ─── Attribute values (mf-1m8f) ──────────────────────────────────────

#[test]
fn attribute_with_literal_value_succeeds() {
    compile_str("{ $x :string @locale=en }").expect("literal attribute value");
}

#[test]
fn attribute_variable_value_rejected() {
    // Per ABNF, attribute values must be literals, not variables
    compile_str("{ $x :string @locale=$y }").expect_err("variable attribute value must fail");
}

#[test]
fn attribute_without_value_succeeds() {
    compile_str("{ $x :string @flag }").expect("attribute without value");
}

// ─── Standalone quoted-pattern (mf-fm5e) ─────────────────────────────

#[test]
fn standalone_quoted_pattern() {
    let bytes = compile_str("{{Hello}}").expect("standalone quoted pattern");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello");
}

#[test]
fn standalone_quoted_pattern_with_declarations() {
    compile_str(".input {$x} {{Hello {$x}}}").expect("declarations + quoted pattern");
}

#[test]
fn standalone_quoted_pattern_with_expression() {
    let bytes = compile_str("{{Hello {$name}}}").expect("quoted pattern with expression");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World");
}
