// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Public compiler entry points.

use std::collections::{BTreeMap, BTreeSet};

use message_format_runtime::schema::{self, FuncEntry, MessageEntry};

use crate::manifest::{
    FunctionManifest, FunctionOperandKind, FunctionSchema, FunctionSelectorKeyKind,
};
use crate::resource::{MessageResource, ResourceInput, ResourceSpan};
use crate::semantic::{
    CallExpr, DetachedSourceSpan, FunctionOption, FunctionOptionValue, FunctionSpec, Message,
    Operand, OperandLiteralKind, Part, SelectArm, SelectExpr, SourceId, SourceInfo, SourceKind,
    SourceSpan,
};
use crate::syntax::ident::{canonicalize_identifier, is_identifier, strip_bidi_controls};
use crate::syntax::literal::{
    decode_text_fragment, ensure_well_formed_quoted_pattern_body, parse_literal_text,
    parse_number_literal, validate_balanced_braces,
};
mod encoder;
mod error;
mod frontend;
mod interning;
mod lowering;

use encoder::{encode_catalog, sort_messages};
pub use error::{CompileError, DiagnosticContext};
use frontend::parse_single_message_with_source;
use interning::{
    collect_functions, collect_strings, escape_fallback_literal, function_dynamic_options,
};
use lowering::lower_parts;

/// Compile-time behavior options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompileOptions {
    /// Whether bare placeholder expressions are implicitly treated as `:string`.
    ///
    /// When `true`, expressions like `{ $name }` that have no explicit function
    /// annotation are lowered as if they were `{ $name :string }`. This enables
    /// default bidi-isolation behavior for those
    /// interpolations.
    ///
    /// This option does not rewrite explicitly annotated expressions such as
    /// `{ $name :number }`, selectors, or markup expressions.
    ///
    /// Default: `false`.
    pub default_bidi_isolation: bool,
}

/// Explicitly keyed MF2 message input for multi-source compilation.
#[derive(Debug, Clone)]
pub struct CompileInput<'a> {
    /// Human-facing source label used for provenance.
    pub name: &'a str,
    /// Explicit message id assigned to this MF2 message body.
    pub message_id: &'a str,
    /// Source text to compile.
    pub source: &'a str,
    /// Logical source kind.
    pub kind: SourceKind,
}

/// One message-to-source entry in an emitted source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSource {
    /// Stable message id.
    pub message_id: String,
    /// Optional container/input provenance for this message.
    pub origin: Option<SourceSpan>,
    /// Optional upstream/original provenance supplied by a resource adapter.
    pub upstream_origin: Option<DetachedSourceSpan>,
}

/// Source provenance emitted alongside a compiled catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceMap {
    /// Registered source table.
    pub sources: Vec<SourceInfo>,
    /// Message-level provenance entries.
    pub messages: Vec<MessageSource>,
}

/// Compiled catalog plus optional provenance sidecar data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledCatalog {
    /// Encoded binary catalog payload.
    pub bytes: Vec<u8>,
    /// Source provenance collected during compilation.
    pub source_map: SourceMap,
}

/// Build-time error that can attach one logical input source to a compiler error.
#[derive(Debug)]
pub struct BuildError {
    /// Diagnostic severity.
    pub severity: DiagnosticSeverity,
    /// Optional container/message context used for resource-backed diagnostics.
    pub context: Option<Box<BuildErrorContext>>,
    /// Underlying compiler failure.
    pub error: Box<CompileError>,
}

/// Severity level for bulk compile diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// Fatal diagnostic that prevents catalog emission.
    Error,
    /// Non-fatal warning.
    Warning,
    /// Informational note.
    Note,
    /// Suggested follow-up action.
    Help,
}

/// Bulk compile result plus any collected diagnostics.
#[derive(Debug, Default)]
pub struct CompileReport {
    /// Compiled catalog when no fatal diagnostics blocked emission.
    pub compiled: Option<CompiledCatalog>,
    /// Diagnostics collected during compilation.
    pub diagnostics: Vec<BuildError>,
}

/// Additional source/message context used when rendering build errors.
#[derive(Debug, Clone)]
pub struct BuildErrorContext {
    /// Source being processed when the error occurred, if known.
    pub source: SourceInfo,
    /// Explicit message id being compiled, if known.
    pub message_id: String,
    /// Original MF2 message body for nested diagnostic rendering, if known.
    pub message_source: Option<String>,
    /// Container-level message origin within the source input, if known.
    pub message_origin: Option<SourceSpan>,
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

impl core::fmt::Display for CompileReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

impl std::error::Error for CompileReport {}

impl BuildError {
    fn error(context: Option<Box<BuildErrorContext>>, error: CompileError) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            context,
            error: Box::new(error),
        }
    }

    /// Render this build error with container-aware context when available.
    #[must_use]
    pub fn render(&self) -> String {
        let Some(context) = self.context.as_deref() else {
            return self.render_without_message_context();
        };
        let Some(message_source) = &context.message_source else {
            return self.render_without_message_context();
        };

        if self.should_render_nested_message_diagnostic(context) {
            return self.render_nested_message_diagnostic(context, message_source);
        }

        self.error
            .render_with_source_label(message_source, &context.source.name)
    }

    fn render_without_message_context(&self) -> String {
        if let Some(context) = self.context.as_deref() {
            let detail = self.error.render_brief();
            if let Some(origin) = &context.message_origin {
                if let (Some(line), Some(column)) = (origin.line, origin.column) {
                    return format!(
                        "{}:{}:{}: message {:?}: {}",
                        context.source.name, line, column, context.message_id, detail
                    );
                }
            }
            format!(
                "{}: message {:?}: {}",
                context.source.name, context.message_id, detail
            )
        } else {
            self.error.to_string()
        }
    }

    fn should_render_nested_message_diagnostic(&self, context: &BuildErrorContext) -> bool {
        if context.source.kind != SourceKind::MessageFormat {
            return true;
        }
        let Some(origin) = &context.message_origin else {
            return false;
        };
        origin.byte_start != 0 || origin.line != Some(1) || origin.column != Some(1)
    }

    fn render_nested_message_diagnostic(
        &self,
        context: &BuildErrorContext,
        message_source: &str,
    ) -> String {
        let mut out = String::new();
        out.push_str(&self.render_container_header(context));
        out.push('\n');
        out.push('\n');
        let label = self.message_label(context);
        out.push_str(&self.error.render_with_source_label(message_source, &label));
        out
    }

    fn render_container_header(&self, context: &BuildErrorContext) -> String {
        let title = self
            .error
            .diagnostic_context()
            .map_or_else(|| self.error.to_string(), |ctx| ctx.title.to_string());
        let message = context.message_id.as_str();

        if let Some(origin) = &context.message_origin {
            if let (Some(line), Some(column)) = (origin.line, origin.column) {
                return format!(
                    "{}:{}:{}: {} in message {:?}",
                    context.source.name, line, column, title, message
                );
            }
        }

        format!(
            "{}: {} in message {:?}",
            context.source.name, title, message
        )
    }

    fn message_label(&self, context: &BuildErrorContext) -> String {
        format!("message {:?}", context.message_id)
    }
}

impl CompileReport {
    /// Construct a successful bulk compile report.
    #[must_use]
    pub fn success(compiled: CompiledCatalog, diagnostics: Vec<BuildError>) -> Self {
        Self {
            compiled: Some(compiled),
            diagnostics,
        }
    }

    /// Construct a failing bulk compile report.
    #[must_use]
    pub fn failure(diagnostics: Vec<BuildError>) -> Self {
        Self {
            compiled: None,
            diagnostics,
        }
    }

    /// Whether any fatal diagnostics were emitted.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diag| diag.severity == DiagnosticSeverity::Error)
    }

    /// Render all collected diagnostics separated by blank lines.
    #[must_use]
    pub fn render(&self) -> String {
        self.diagnostics
            .iter()
            .map(BuildError::render)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Convert this report into a plain catalog result.
    pub fn into_result(self) -> Result<CompiledCatalog, Vec<BuildError>> {
        match self.compiled {
            Some(compiled) if !self.has_errors() => Ok(compiled),
            _ => Err(self.diagnostics),
        }
    }
}

/// Compiler-side builder for assembling one catalog from multiple text or structured inputs.
#[derive(Debug, Clone)]
pub struct CatalogBuilder {
    options: CompileOptions,
    function_manifest: Option<FunctionManifest>,
    sources: Vec<SourceInfo>,
    messages: Vec<Message>,
    next_source_id: u32,
}

impl Default for CatalogBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CatalogBuilder {
    /// Create an empty builder with default compile options.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(CompileOptions::default())
    }

    /// Create an empty builder with explicit compile options.
    #[must_use]
    pub fn with_options(options: CompileOptions) -> Self {
        Self {
            options,
            function_manifest: None,
            sources: Vec::new(),
            messages: Vec::new(),
            next_source_id: 0,
        }
    }

    /// Attach a function manifest used for compile-time function validation.
    pub fn set_function_manifest(&mut self, manifest: FunctionManifest) {
        self.function_manifest = Some(manifest);
    }

    /// Register one logical source for later structured message insertion.
    pub fn register_source(
        &mut self,
        name: impl Into<String>,
        kind: SourceKind,
    ) -> Result<SourceId, CompileError> {
        let id = SourceId(self.next_source_id);
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .ok_or(CompileError::size_overflow("source id"))?;
        self.sources.push(SourceInfo {
            id,
            name: name.into(),
            kind,
        });
        Ok(id)
    }

    /// Parse one MF2 message body and add it under its explicit message id.
    pub fn add_input(&mut self, input: CompileInput<'_>) -> Result<SourceId, BuildError> {
        let source_id = self
            .register_source(input.name, input.kind)
            .map_err(|error| BuildError::error(None, error))?;
        let source_info = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .cloned();
        let mut message =
            parse_single_message_with_source(input.source, self.options, Some(source_id)).map_err(
                |error| {
                    BuildError::error(
                        source_info.map(|source| {
                            Box::new(BuildErrorContext {
                                source,
                                message_id: input.message_id.to_string(),
                                message_source: Some(input.source.to_string()),
                                message_origin: Some(SourceSpan {
                                    source_id,
                                    byte_start: 0,
                                    byte_end: input.source.len(),
                                    line: Some(1),
                                    column: Some(1),
                                }),
                            })
                        }),
                        error,
                    )
                },
            )?;
        message.id = input.message_id.to_string();
        self.add_message(message)?;
        Ok(source_id)
    }

    /// Parse one resource/container input made up of named MF2 message bodies.
    pub fn add_resource_input(&mut self, input: ResourceInput) -> Result<SourceId, BuildError> {
        let source_name = input.name.clone();
        let source_kind = input.kind.clone();
        let source_id = self
            .register_source(input.name, input.kind)
            .map_err(|error| BuildError::error(None, error))?;

        let source_info = SourceInfo {
            id: source_id,
            name: source_name.clone(),
            kind: source_kind.clone(),
        };

        for resource in input.resources {
            let message =
                parse_message_resource(&resource, self.options, source_id).map_err(|error| {
                    BuildError::error(
                        Some(Box::new(BuildErrorContext {
                            source: source_info.clone(),
                            message_id: resource.id.clone(),
                            message_source: Some(resource.source.clone()),
                            message_origin: resource
                                .origin
                                .as_ref()
                                .map(|origin| resource_span_to_source_span(origin, source_id)),
                        })),
                        error,
                    )
                })?;
            self.add_message(message)?;
        }

        Ok(source_id)
    }

    /// Add one already-structured semantic message.
    pub fn add_message(&mut self, message: Message) -> Result<(), BuildError> {
        ensure_source_registered(&self.sources, &message)?;
        self.messages.push(message);
        Ok(())
    }

    /// Add multiple already-structured semantic messages.
    pub fn add_messages<I>(&mut self, messages: I) -> Result<(), BuildError>
    where
        I: IntoIterator<Item = Message>,
    {
        for message in messages {
            self.add_message(message)?;
        }
        Ok(())
    }

    /// Compile into a binary catalog payload plus provenance sidecar data.
    pub fn compile(self) -> CompileReport {
        let Self {
            options,
            sources,
            messages,
            function_manifest,
            next_source_id,
        } = self;
        compile_builder_report(
            Self {
                options,
                function_manifest,
                sources,
                messages,
                next_source_id,
            },
            Vec::new(),
        )
    }

    /// Compile into a binary catalog payload only.
    pub fn compile_bytes(self) -> Result<Vec<u8>, BuildError> {
        self.compile()
            .into_result()
            .map(|compiled| compiled.bytes)
            .map_err(|mut diagnostics| {
                diagnostics.drain(..1).next().unwrap_or_else(|| {
                    BuildError::error(
                        None,
                        CompileError::internal("build error collection unexpectedly empty"),
                    )
                })
            })
    }
}

/// Compile one MF2 message source text into a binary catalog payload.
///
/// ```rust
/// use message_format_compiler::compile_str;
///
/// let bytes = compile_str("Hello { $name }!").unwrap();
/// assert!(!bytes.is_empty());
/// ```
pub fn compile_str(source: &str) -> Result<Vec<u8>, CompileError> {
    compile(source, CompileOptions::default())
}

/// Compile one MF2 message source text into a binary catalog payload with explicit options.
///
/// ```rust
/// use message_format_compiler::{CompileOptions, compile};
///
/// let options = CompileOptions {
///     default_bidi_isolation: true,
/// };
/// let bytes = compile("{ $name }", options).unwrap();
/// assert!(!bytes.is_empty());
/// ```
pub fn compile(source: &str, options: CompileOptions) -> Result<Vec<u8>, CompileError> {
    let message = parse_single_message_with_source(source, options, Some(SourceId(0)))?;
    compile_parsed_messages(vec![message], None)
}

/// Compile source text using a function manifest for custom compile-time validation.
///
/// This keeps the compiler host-agnostic: the manifest describes which custom
/// functions and options are valid, but it does not execute them.
pub fn compile_with_manifest(
    source: &str,
    options: CompileOptions,
    manifest: &FunctionManifest,
) -> Result<Vec<u8>, CompileError> {
    let message = parse_single_message_with_source(source, options, Some(SourceId(0)))?;
    compile_parsed_messages(vec![message], Some(manifest))
}

/// Compile multiple explicitly keyed MF2 message inputs into one catalog with provenance.
pub fn compile_inputs<'a>(
    inputs: impl IntoIterator<Item = CompileInput<'a>>,
    options: CompileOptions,
) -> CompileReport {
    let mut builder = CatalogBuilder::with_options(options);
    let mut diagnostics = Vec::new();
    for input in inputs {
        if let Err(error) = builder.add_input(input) {
            diagnostics.push(error);
        }
    }
    compile_builder_report(builder, diagnostics)
}

/// Compile multiple resource/container inputs into one catalog with provenance.
pub fn compile_resources(
    inputs: impl IntoIterator<Item = ResourceInput>,
    options: CompileOptions,
) -> CompileReport {
    let mut builder = CatalogBuilder::with_options(options);
    let mut diagnostics = Vec::new();
    for input in inputs {
        collect_resource_input(&mut builder, input, &mut diagnostics);
    }
    compile_builder_report(builder, diagnostics)
}

/// Compile multiple explicitly keyed MF2 message inputs into one catalog using a function manifest.
pub fn compile_inputs_with_manifest<'a>(
    inputs: impl IntoIterator<Item = CompileInput<'a>>,
    options: CompileOptions,
    manifest: &FunctionManifest,
) -> CompileReport {
    let mut builder = CatalogBuilder::with_options(options);
    builder.set_function_manifest(manifest.clone());
    let mut diagnostics = Vec::new();
    for input in inputs {
        if let Err(error) = builder.add_input(input) {
            diagnostics.push(error);
        }
    }
    compile_builder_report(builder, diagnostics)
}

/// Compile multiple resource/container inputs into one catalog using a function manifest.
pub fn compile_resources_with_manifest(
    inputs: impl IntoIterator<Item = ResourceInput>,
    options: CompileOptions,
    manifest: &FunctionManifest,
) -> CompileReport {
    let mut builder = CatalogBuilder::with_options(options);
    builder.set_function_manifest(manifest.clone());
    let mut diagnostics = Vec::new();
    for input in inputs {
        collect_resource_input(&mut builder, input, &mut diagnostics);
    }
    compile_builder_report(builder, diagnostics)
}

fn collect_resource_input(
    builder: &mut CatalogBuilder,
    input: ResourceInput,
    diagnostics: &mut Vec<BuildError>,
) {
    let source_name = input.name.clone();
    let source_kind = input.kind.clone();
    let source_id = match builder.register_source(input.name, input.kind) {
        Ok(source_id) => source_id,
        Err(error) => {
            diagnostics.push(BuildError::error(None, error));
            return;
        }
    };

    let source_info = SourceInfo {
        id: source_id,
        name: source_name,
        kind: source_kind,
    };

    for resource in input.resources {
        match parse_message_resource(&resource, builder.options, source_id) {
            Ok(message) => {
                if let Err(error) = builder.add_message(message) {
                    diagnostics.push(error);
                }
            }
            Err(error) => diagnostics.push(BuildError::error(
                Some(Box::new(BuildErrorContext {
                    source: source_info.clone(),
                    message_id: resource.id.clone(),
                    message_source: Some(resource.source.clone()),
                    message_origin: resource
                        .origin
                        .as_ref()
                        .map(|origin| resource_span_to_source_span(origin, source_id)),
                })),
                error,
            )),
        }
    }
}

fn compile_builder_report(
    builder: CatalogBuilder,
    mut diagnostics: Vec<BuildError>,
) -> CompileReport {
    let CatalogBuilder {
        options: _,
        function_manifest,
        sources,
        messages,
        next_source_id: _,
    } = builder;

    diagnostics.extend(collect_build_errors(
        &messages,
        &sources,
        function_manifest.as_ref(),
    ));

    if diagnostics.is_empty() {
        match encode_messages(&messages) {
            Ok(bytes) => CompileReport::success(
                CompiledCatalog {
                    bytes,
                    source_map: build_source_map(messages, sources),
                },
                diagnostics,
            ),
            Err(error) => CompileReport::failure(vec![build_error_for_compile(
                messages.as_slice(),
                &sources,
                error,
            )]),
        }
    } else {
        CompileReport::failure(diagnostics)
    }
}

fn compile_parsed_messages(
    messages: Vec<Message>,
    function_manifest: Option<&FunctionManifest>,
) -> Result<Vec<u8>, CompileError> {
    validate_messages(&messages, function_manifest)?;
    encode_messages(&messages)
}

fn validate_messages(
    messages: &[Message],
    function_manifest: Option<&FunctionManifest>,
) -> Result<(), CompileError> {
    ensure_unique_message_ids(messages)?;
    validate_messages_against_builtins(messages)?;
    if let Some(manifest) = function_manifest {
        validate_messages_against_manifest(messages, manifest)?;
    }
    Ok(())
}

fn collect_build_errors(
    messages: &[Message],
    sources: &[SourceInfo],
    function_manifest: Option<&FunctionManifest>,
) -> Vec<BuildError> {
    let mut diagnostics = Vec::new();

    if let Err(error) = ensure_unique_message_ids(messages) {
        diagnostics.push(build_error_for_compile(messages, sources, error));
    }

    for error in collect_builtin_validation_errors(messages) {
        diagnostics.push(build_error_for_compile(messages, sources, error));
    }

    if let Some(manifest) = function_manifest {
        for error in collect_manifest_validation_errors(messages, manifest) {
            diagnostics.push(build_error_for_compile(messages, sources, error));
        }
    }

    diagnostics
}

fn encode_messages(messages: &[Message]) -> Result<Vec<u8>, CompileError> {
    let mut all_strings = BTreeSet::new();
    collect_strings(messages, &mut all_strings);

    let mut string_map = BTreeMap::new();
    for (idx, key) in all_strings.into_iter().enumerate() {
        let id = u32::try_from(idx).map_err(|_| CompileError::TooManyStrings)?;
        string_map.insert(key, id);
    }

    let mut strings = vec![""; string_map.len()];
    for (value, id) in &string_map {
        strings[*id as usize] = value;
    }

    let (collected_funcs, func_map) = collect_functions(messages)?;

    let func_entries: Vec<FuncEntry> = collected_funcs
        .iter()
        .map(|cf| {
            let name_str_id = *string_map
                .get(&cf.name)
                .expect("function name must be in string pool");
            let static_options = cf
                .static_options
                .iter()
                .map(|(k, v)| {
                    let key_id = *string_map
                        .get(k)
                        .expect("option key must be in string pool");
                    let val_id = *string_map
                        .get(v)
                        .expect("option value must be in string pool");
                    (key_id, val_id)
                })
                .collect();
            FuncEntry {
                name_str_id,
                static_options,
            }
        })
        .collect();

    let mut literals = String::new();
    let mut code = Vec::new();
    let mut entries = Vec::new();

    for message in messages {
        let entry_pc = u32::try_from(code.len())
            .map_err(|_| CompileError::size_overflow("message entry program counter"))?;
        let name_str_id = *string_map
            .get(&message.id)
            .ok_or(CompileError::internal("missing interned message id"))?;
        entries.push(MessageEntry {
            name_str_id,
            entry_pc,
        });

        lower_parts(
            &message.parts,
            &string_map,
            &func_map,
            &mut literals,
            &mut code,
        )?;
        code.push(schema::OP_HALT);
    }
    sort_messages(&mut entries);

    encode_catalog(&strings, &literals, &entries, &code, &func_entries)
}

fn ensure_unique_message_ids(messages: &[Message]) -> Result<(), CompileError> {
    let mut seen = BTreeMap::<&str, Option<SourceSpan>>::new();
    for message in messages {
        if let Some(first_origin) = seen.insert(message.id.as_str(), message.origin.clone()) {
            return Err(CompileError::duplicate_message_id(
                &message.id,
                first_origin,
                message.origin.clone(),
            ));
        }
    }
    Ok(())
}

fn ensure_source_registered(sources: &[SourceInfo], message: &Message) -> Result<(), BuildError> {
    let Some(origin) = &message.origin else {
        return Ok(());
    };
    if sources.iter().any(|source| source.id == origin.source_id) {
        return Ok(());
    }
    Err(BuildError::error(
        None,
        CompileError::internal(format!(
            "message {:?} references unregistered source id {}",
            message.id, origin.source_id.0
        )),
    ))
}

fn build_source_map(messages: Vec<Message>, sources: Vec<SourceInfo>) -> SourceMap {
    SourceMap {
        sources,
        messages: messages
            .into_iter()
            .map(|message| MessageSource {
                message_id: message.id,
                origin: message.origin,
                upstream_origin: message.upstream_origin,
            })
            .collect(),
    }
}

fn parse_message_resource(
    resource: &MessageResource,
    options: CompileOptions,
    source_id: SourceId,
) -> Result<Message, CompileError> {
    let mut message = parse_single_message_with_source(&resource.source, options, Some(source_id))?;
    message.id = resource.id.clone();
    message.origin = resource
        .origin
        .as_ref()
        .map(|origin| resource_span_to_source_span(origin, source_id));
    message.upstream_origin = resource.upstream_origin.clone();
    Ok(message)
}

fn resource_span_to_source_span(origin: &ResourceSpan, source_id: SourceId) -> SourceSpan {
    SourceSpan {
        source_id,
        byte_start: origin.byte_start,
        byte_end: origin.byte_end,
        line: origin.line,
        column: origin.column,
    }
}

fn validate_messages_against_builtins(messages: &[Message]) -> Result<(), CompileError> {
    if let Some(error) = collect_builtin_validation_errors(messages)
        .into_iter()
        .next()
    {
        Err(error)
    } else {
        Ok(())
    }
}

fn collect_builtin_validation_errors(messages: &[Message]) -> Vec<CompileError> {
    let mut errors = Vec::new();
    for message in messages {
        collect_builtin_part_errors(&message.parts, message, &mut errors);
    }
    errors
}

fn collect_builtin_part_errors(parts: &[Part], message: &Message, errors: &mut Vec<CompileError>) {
    for part in parts {
        match part {
            Part::Call(CallExpr {
                operand: _, func, ..
            }) => {
                collect_builtin_function_errors(func, message, errors);
            }
            Part::Select(SelectExpr {
                selector,
                arms,
                default,
            }) => {
                if let crate::semantic::SelectorExpr::Call { func, .. } = selector {
                    collect_builtin_function_errors(func, message, errors);
                }
                for arm in arms {
                    collect_builtin_part_errors(&arm.parts, message, errors);
                }
                collect_builtin_part_errors(default, message, errors);
            }
            Part::MarkupOpen { .. }
            | Part::MarkupClose { .. }
            | Part::Text(_)
            | Part::Literal(_)
            | Part::Var(_) => {}
        }
    }
}

fn collect_builtin_function_errors(
    func: &FunctionSpec,
    message: &Message,
    errors: &mut Vec<CompileError>,
) {
    for option in &func.options {
        let Some(value) = builtin_option_literal_domain(&func.name, &option.key) else {
            continue;
        };
        let FunctionOptionValue::Literal(found) = &option.value else {
            continue;
        };
        if value.iter().any(|candidate| candidate == found) {
            continue;
        }
        errors.push(CompileError::invalid_builtin_option_value(
            &func.name,
            &option.key,
            join_expected_values(value),
            found,
            &message.id,
            func.origin.clone().or_else(|| message.origin.clone()),
        ));
    }
}

fn builtin_option_literal_domain(function: &str, option: &str) -> Option<&'static [&'static str]> {
    match (function, option) {
        ("string", "u:dir") => Some(&["ltr", "rtl", "auto"]),
        ("number" | "integer", "signDisplay") => Some(&["auto", "always", "never"]),
        ("number" | "integer", "style") => Some(&["percent"]),
        ("number" | "integer", "select") => Some(&["exact", "plural", "ordinal"]),
        ("number" | "integer", "notation") => Some(&["scientific"]),
        ("number" | "integer", "useGrouping") => Some(&["auto", "always", "never", "min2"]),
        ("date", "style" | "dateStyle") => Some(&["short", "medium", "long", "full"]),
        ("time", "style" | "timeStyle") => Some(&["short", "medium", "long", "full"]),
        ("datetime", "style" | "dateStyle" | "timeStyle") => {
            Some(&["short", "medium", "long", "full"])
        }
        _ => None,
    }
}

fn join_expected_values(values: &'static [&'static str]) -> &'static str {
    match values {
        ["ltr", "rtl", "auto"] => "\"ltr\", \"rtl\", or \"auto\"",
        ["auto", "always", "never"] => "\"auto\", \"always\", or \"never\"",
        ["percent"] => "\"percent\"",
        ["exact", "plural", "ordinal"] => "\"exact\", \"plural\", or \"ordinal\"",
        ["scientific"] => "\"scientific\"",
        ["auto", "always", "never", "min2"] => "\"auto\", \"always\", \"never\", or \"min2\"",
        ["short", "medium", "long", "full"] => "\"short\", \"medium\", \"long\", or \"full\"",
        _ => "the documented builtin values",
    }
}

fn validate_messages_against_manifest(
    messages: &[Message],
    manifest: &FunctionManifest,
) -> Result<(), CompileError> {
    if let Some(error) = collect_manifest_validation_errors(messages, manifest)
        .into_iter()
        .next()
    {
        Err(error)
    } else {
        Ok(())
    }
}

fn collect_manifest_validation_errors(
    messages: &[Message],
    manifest: &FunctionManifest,
) -> Vec<CompileError> {
    let mut errors = Vec::new();
    for message in messages {
        collect_manifest_part_errors(&message.parts, manifest, message, &mut errors);
    }
    errors
}

fn collect_manifest_part_errors(
    parts: &[Part],
    manifest: &FunctionManifest,
    message: &Message,
    errors: &mut Vec<CompileError>,
) {
    for part in parts {
        match part {
            Part::Call(CallExpr { operand, func, .. }) => {
                collect_function_spec_errors_into(
                    func,
                    Some(operand),
                    FunctionUse::Format,
                    manifest,
                    message,
                    errors,
                );
            }
            Part::Select(SelectExpr {
                selector,
                arms,
                default,
            }) => {
                collect_selector_manifest_errors(selector, arms, manifest, message, errors);
                for arm in arms {
                    collect_manifest_part_errors(&arm.parts, manifest, message, errors);
                }
                collect_manifest_part_errors(default, manifest, message, errors);
            }
            Part::Text(_) | Part::Literal(_) | Part::Var(_) => {}
            Part::MarkupOpen { name, options } | Part::MarkupClose { name, options } => {
                collect_markup_manifest_errors_into(name, options, manifest, message, errors);
            }
        }
    }
}

fn collect_markup_manifest_errors_into(
    name: &str,
    options: &[FunctionOption],
    manifest: &FunctionManifest,
    message: &Message,
    errors: &mut Vec<CompileError>,
) {
    let Some(schema) = manifest.get(name) else {
        errors.push(CompileError::unknown_function_manifest(
            name,
            &message.id,
            message.origin.clone(),
        ));
        return;
    };
    if !schema.allows_format {
        errors.push(CompileError::unsupported_function_usage(
            &schema.name,
            FunctionUse::Format.as_str(),
            &message.id,
            message.origin.clone(),
        ));
        return;
    }
    for option in options {
        if let Err(error) = validate_markup_option(schema, option, message) {
            errors.push(error);
        }
    }
    if let Err(error) = validate_required_markup_options(schema, options, message) {
        errors.push(error);
    }
}

fn validate_markup_option(
    schema: &FunctionSchema,
    option: &FunctionOption,
    message: &Message,
) -> Result<(), CompileError> {
    let Some(option_schema) = schema.option_schema(&option.key) else {
        if schema.allow_unknown_options {
            return Ok(());
        }
        return Err(CompileError::unknown_function_option(
            &schema.name,
            &option.key,
            &message.id,
            message.origin.clone(),
        ));
    };
    let accepts = match option.value {
        FunctionOptionValue::Literal(_) => option_schema.value_kind.accepts_literal(),
        FunctionOptionValue::Var(_) => option_schema.value_kind.accepts_variable(),
    };
    if accepts {
        return Ok(());
    }
    Err(CompileError::invalid_function_option_value(
        &schema.name,
        &option.key,
        option_schema.value_kind,
        &message.id,
        message.origin.clone(),
    ))
}

fn validate_required_markup_options(
    schema: &FunctionSchema,
    options: &[FunctionOption],
    message: &Message,
) -> Result<(), CompileError> {
    for option in schema.options().filter(|option| option.required) {
        if options.iter().any(|candidate| candidate.key == option.key) {
            continue;
        }
        return Err(CompileError::missing_function_option(
            &schema.name,
            &option.key,
            &message.id,
            message.origin.clone(),
        ));
    }
    Ok(())
}

fn collect_selector_manifest_errors(
    selector: &crate::semantic::SelectorExpr,
    arms: &[SelectArm],
    manifest: &FunctionManifest,
    message: &Message,
    errors: &mut Vec<CompileError>,
) {
    if let crate::semantic::SelectorExpr::Call { operand, func } = selector {
        collect_function_spec_errors_into(
            func,
            Some(operand),
            FunctionUse::Select,
            manifest,
            message,
            errors,
        );
        let Some(schema) = manifest.get(&func.name) else {
            return;
        };
        for arm in arms {
            if let Err(error) = validate_selector_key(schema, func, arm, message) {
                errors.push(error);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum FunctionUse {
    Format,
    Select,
}

impl FunctionUse {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Select => "select",
        }
    }
}

fn collect_function_spec_errors_into(
    func: &FunctionSpec,
    operand: Option<&Operand>,
    use_site: FunctionUse,
    manifest: &FunctionManifest,
    message: &Message,
    errors: &mut Vec<CompileError>,
) {
    let Some(schema) = manifest.get(&func.name) else {
        errors.push(CompileError::unknown_function_manifest(
            &func.name,
            &message.id,
            func.origin.clone().or_else(|| message.origin.clone()),
        ));
        return;
    };
    if let Err(error) = validate_function_usage(schema, func, use_site, message) {
        errors.push(error);
    }
    if let Err(error) = validate_function_operand(schema, func, operand, message) {
        errors.push(error);
    }
    for option in &func.options {
        if let Err(error) = validate_function_option(schema, func, option, message) {
            errors.push(error);
        }
    }
    if let Err(error) = validate_required_function_options(schema, func, message) {
        errors.push(error);
    }
}

fn validate_function_usage(
    schema: &FunctionSchema,
    func: &FunctionSpec,
    use_site: FunctionUse,
    message: &Message,
) -> Result<(), CompileError> {
    let allowed = match use_site {
        FunctionUse::Format => schema.allows_format,
        FunctionUse::Select => schema.allows_select,
    };
    if allowed {
        return Ok(());
    }
    Err(CompileError::unsupported_function_usage(
        &schema.name,
        use_site.as_str(),
        &message.id,
        func.origin.clone().or_else(|| message.origin.clone()),
    ))
}

fn validate_function_option(
    schema: &FunctionSchema,
    func: &FunctionSpec,
    option: &FunctionOption,
    message: &Message,
) -> Result<(), CompileError> {
    let Some(option_schema) = schema.option_schema(&option.key) else {
        if schema.allow_unknown_options {
            return Ok(());
        }
        return Err(CompileError::unknown_function_option(
            &schema.name,
            &option.key,
            &message.id,
            func.origin.clone().or_else(|| message.origin.clone()),
        ));
    };
    let accepts = match option.value {
        FunctionOptionValue::Literal(_) => option_schema.value_kind.accepts_literal(),
        FunctionOptionValue::Var(_) => option_schema.value_kind.accepts_variable(),
    };
    if accepts {
        return Ok(());
    }
    Err(CompileError::invalid_function_option_value(
        &schema.name,
        &option.key,
        option_schema.value_kind,
        &message.id,
        func.origin.clone().or_else(|| message.origin.clone()),
    ))
}

fn validate_required_function_options(
    schema: &FunctionSchema,
    func: &FunctionSpec,
    message: &Message,
) -> Result<(), CompileError> {
    for option in schema.options().filter(|option| option.required) {
        if func
            .options
            .iter()
            .any(|candidate| candidate.key == option.key)
        {
            continue;
        }
        return Err(CompileError::missing_function_option(
            &schema.name,
            &option.key,
            &message.id,
            func.origin.clone().or_else(|| message.origin.clone()),
        ));
    }
    Ok(())
}

fn validate_function_operand(
    schema: &FunctionSchema,
    func: &FunctionSpec,
    operand: Option<&Operand>,
    message: &Message,
) -> Result<(), CompileError> {
    let operand = match schema.operand_kind {
        FunctionOperandKind::Any => return Ok(()),
        FunctionOperandKind::String => match operand {
            Some(Operand::Literal { value, kind }) => {
                return if *kind == OperandLiteralKind::String {
                    Ok(())
                } else {
                    Err(CompileError::invalid_function_operand(
                        &schema.name,
                        schema.operand_kind,
                        format!("{value:?}"),
                        &message.id,
                        func.origin.clone().or_else(|| message.origin.clone()),
                    ))
                };
            }
            Some(Operand::Var(_)) | None => None,
        },
        FunctionOperandKind::Number => match operand {
            Some(Operand::Literal { value, kind }) => {
                return if *kind == OperandLiteralKind::Number {
                    Ok(())
                } else {
                    Err(CompileError::invalid_function_operand(
                        &schema.name,
                        schema.operand_kind,
                        format!("{value:?}"),
                        &message.id,
                        func.origin.clone().or_else(|| message.origin.clone()),
                    ))
                };
            }
            Some(Operand::Var(_)) | None => None,
        },
    };
    let Some(operand) = operand else {
        return Ok(());
    };
    if parse_number_literal(operand).is_some() {
        return Ok(());
    }
    Err(CompileError::invalid_function_operand(
        &schema.name,
        schema.operand_kind,
        format!("{operand:?}"),
        &message.id,
        func.origin.clone().or_else(|| message.origin.clone()),
    ))
}

fn source_for_error(sources: &[SourceInfo], error: &CompileError) -> Option<SourceInfo> {
    let source_id = error.primary_source_id()?;
    sources
        .iter()
        .find(|source| source.id == source_id)
        .cloned()
}

fn message_context_for_error<'a>(
    messages: &'a [Message],
    error: &CompileError,
) -> Option<&'a Message> {
    let message_id = error.primary_message_id()?;
    let source_id = error.primary_source_id();
    messages.iter().find(|message| {
        if message.id != message_id {
            return false;
        }
        match source_id {
            Some(source_id) => message
                .origin
                .as_ref()
                .is_some_and(|origin| origin.source_id == source_id),
            None => true,
        }
    })
}

fn build_error_for_compile(
    messages: &[Message],
    sources: &[SourceInfo],
    error: CompileError,
) -> BuildError {
    let message = message_context_for_error(messages, &error);
    BuildError {
        severity: DiagnosticSeverity::Error,
        context: message.and_then(|message| {
            source_for_error(sources, &error).map(|source| {
                Box::new(BuildErrorContext {
                    source,
                    message_id: message.id.clone(),
                    message_source: message.message_source.clone(),
                    message_origin: message.origin.clone(),
                })
            })
        }),
        error: Box::new(error),
    }
}

fn validate_selector_key(
    schema: &FunctionSchema,
    func: &FunctionSpec,
    arm: &SelectArm,
    message: &Message,
) -> Result<(), CompileError> {
    if selector_key_kind_accepts(schema.selector_keys, &arm.key) {
        return Ok(());
    }
    Err(CompileError::invalid_variant_key_source(
        arm.origin
            .clone()
            .or_else(|| func.origin.clone())
            .or_else(|| message.origin.clone()),
        selector_key_kind_expectation(schema.selector_keys),
        &arm.key,
    ))
}

fn selector_key_kind_accepts(kind: FunctionSelectorKeyKind, key: &str) -> bool {
    match kind {
        FunctionSelectorKeyKind::Any => true,
        FunctionSelectorKeyKind::Identifier => is_identifier(key),
        FunctionSelectorKeyKind::Number => parse_number_literal(key).is_some(),
        FunctionSelectorKeyKind::PluralCategory => is_cldr_plural_category(key),
        FunctionSelectorKeyKind::NumberOrPluralCategory => {
            parse_number_literal(key).is_some() || is_cldr_plural_category(key)
        }
    }
}

fn selector_key_kind_expectation(kind: FunctionSelectorKeyKind) -> &'static str {
    match kind {
        FunctionSelectorKeyKind::Any => "documented selector key",
        FunctionSelectorKeyKind::Identifier => "MF2 identifier",
        FunctionSelectorKeyKind::Number => "MF2 number literal",
        FunctionSelectorKeyKind::PluralCategory => "CLDR plural category",
        FunctionSelectorKeyKind::NumberOrPluralCategory => {
            "CLDR plural category or MF2 number literal"
        }
    }
}

fn is_cldr_plural_category(key: &str) -> bool {
    matches!(key, "zero" | "one" | "two" | "few" | "many" | "other")
}

#[cfg(test)]
mod tests;
