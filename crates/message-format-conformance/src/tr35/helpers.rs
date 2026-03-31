// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::runtime_helpers;
use message_format::{
    Locale,
    compiler::{CompileError, CompileOptions, compile, compile_str},
    runtime::{BuiltinHost, Catalog, FormatError, Formatter, MessageFunctionError, Value},
};

/// Return source as-is for compilation. Single MF2 messages are auto-assigned
/// id "main" by the compiler's single-message path, so no wrapping is needed.
fn wrap(source: &str) -> String {
    source.to_string()
}

/// Compile source text, returning the catalog bytes.
fn compile_source(source: &str) -> Result<Vec<u8>, CompileError> {
    compile_str(&wrap(source))
}

/// Compile source text with explicit options.
fn compile_source_with_options(
    source: &str,
    opts: CompileOptions,
) -> Result<Vec<u8>, CompileError> {
    compile(&wrap(source), opts)
}

/// Strip FSI (U+2068) and PDI (U+2069) bidi isolation markers from output.
fn strip_bidi(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn parse_locale(tag: &str) -> Locale {
    tag.parse::<Locale>().expect("locale parse failed")
}

fn resolve_args(catalog: &Catalog, args: &[(&str, Value)]) -> Vec<(u32, Value)> {
    args.iter()
        .filter_map(|(name, value)| catalog.string_id(name).map(|id| (id, value.clone())))
        .collect()
}

fn format_output_with_locale(
    source: &str,
    args: &[(&str, Value)],
    locale: &str,
) -> Result<runtime_helpers::FormatOutput, FormatError> {
    let bytes = compile_source(source).expect("compile failed");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog load failed");
    let locale = parse_locale(locale);
    let host = BuiltinHost::from_catalog(&catalog, &locale).expect("host init failed");
    let mut formatter = Formatter::new(&catalog, host);
    let args = resolve_args(&catalog, args);
    runtime_helpers::format_with_diagnostics_by_id(&mut formatter, "main", &args)
}

fn format_output_with_options_and_locale(
    source: &str,
    args: &[(&str, Value)],
    opts: CompileOptions,
    locale: &str,
) -> Result<runtime_helpers::FormatOutput, FormatError> {
    let bytes = compile_source_with_options(source, opts).expect("compile failed");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog load failed");
    let host =
        BuiltinHost::from_catalog(&catalog, &parse_locale(locale)).expect("host init failed");
    let mut formatter = Formatter::new(&catalog, host);
    let args = resolve_args(&catalog, args);
    runtime_helpers::format_with_diagnostics_by_id(&mut formatter, "main", &args)
}

/// Format a single-message source and return the output string (bidi stripped).
/// Uses `en` locale by default.
pub(super) fn format(source: &str, args: &[(&str, Value)]) -> String {
    format_locale(source, args, "en")
}

/// Format a single-message source and return both output and diagnostics.
pub(super) fn format_output(source: &str, args: &[(&str, Value)]) -> runtime_helpers::FormatOutput {
    format_output_with_locale(source, args, "en").expect("format failed")
}

/// Format with an explicit locale (bidi stripped).
pub(super) fn format_locale(source: &str, args: &[(&str, Value)], locale: &str) -> String {
    let output = format_output_with_locale(source, args, locale).expect("format failed");
    strip_bidi(&output.value)
}

/// Format and return the raw output string (preserving bidi markers).
pub(super) fn format_raw(source: &str, args: &[(&str, Value)]) -> String {
    format_output_with_locale(source, args, "en")
        .expect("format failed")
        .value
}

/// Assert that a source formats to expected output (bidi stripped).
pub(super) fn assert_format(source: &str, args: &[(&str, Value)], expected: &str) {
    let actual = format(source, args);
    assert_eq!(actual, expected, "source: {source}");
}

/// Assert with explicit locale (bidi stripped).
pub(super) fn assert_format_locale(
    source: &str,
    args: &[(&str, Value)],
    locale: &str,
    expected: &str,
) {
    let actual = format_locale(source, args, locale);
    assert_eq!(actual, expected, "source: {source} locale: {locale}");
}

/// Assert that compilation fails and the error satisfies `check`.
pub(super) fn assert_compile_err(source: &str, check: impl Fn(&CompileError) -> bool) {
    let err = compile_source(source).expect_err(&format!("expected compile error for: {source}"));
    assert!(check(&err), "unexpected compile error variant: {err:?}");
}

/// Assert that formatting produces at least one error satisfying `check`.
///
/// With fallback rendering, format always produces output. This function
/// uses `format_with_diagnostics_by_id` to inspect collected errors.
pub(super) fn assert_format_err(
    source: &str,
    args: &[(&str, Value)],
    check: impl Fn(&FormatError) -> bool,
) {
    match format_output_with_locale(source, args, "en") {
        Ok(output) => {
            assert!(
                output.errors.iter().any(&check),
                "expected a matching format error for: {source}: {:?}",
                output.value
            );
        }
        Err(err) => {
            assert!(check(&err), "unexpected format error variant: {err:?}");
        }
    }
}

/// Assert that formatting returns the expected output and reports a matching error.
pub(super) fn assert_format_with_error(
    source: &str,
    args: &[(&str, Value)],
    expected: &str,
    check: impl Fn(&FormatError) -> bool,
) {
    let output = format_output_with_locale(source, args, "en").expect("format failed");
    assert_eq!(strip_bidi(&output.value), expected, "source: {source}");
    assert!(
        output.errors.iter().any(check),
        "expected a matching format error for: {source}: {:?}",
        output.errors
    );
}

/// Assert compilation succeeds (no panic, no error).
#[allow(
    dead_code,
    reason = "Helper is used selectively by conformance modules and feature slices."
)]
pub(super) fn assert_compiles(source: &str) {
    compile_source(source)
        .unwrap_or_else(|e| panic!("expected compile success for: {source}\nerror: {e}"));
}

/// Compile + format with `CompileOptions` (bidi stripped).
#[allow(
    dead_code,
    reason = "Helper is used selectively by conformance modules and feature slices."
)]
pub(super) fn format_with_options(
    source: &str,
    args: &[(&str, Value)],
    opts: CompileOptions,
) -> String {
    let output =
        format_output_with_options_and_locale(source, args, opts, "en").expect("format failed");
    strip_bidi(&output.value)
}

/// Compile + format with `CompileOptions`, returning raw output (preserving bidi).
pub(super) fn format_with_options_raw(
    source: &str,
    args: &[(&str, Value)],
    opts: CompileOptions,
) -> String {
    format_output_with_options_and_locale(source, args, opts, "en")
        .expect("format failed")
        .value
}

pub(super) fn is_syntax_error(error: &CompileError) -> bool {
    matches!(
        error,
        CompileError::InvalidLine { .. }
            | CompileError::InvalidExpr { .. }
            | CompileError::InvalidVar { .. }
    )
}

pub(super) fn is_invalid_expr(error: &CompileError) -> bool {
    matches!(error, CompileError::InvalidExpr { .. })
}

pub(super) fn is_missing_default_arm(error: &CompileError) -> bool {
    matches!(error, CompileError::MissingDefaultArm { .. })
}

pub(super) fn is_duplicate_option_name(error: &CompileError) -> bool {
    matches!(error, CompileError::DuplicateOptionName { .. })
}

pub(super) fn is_duplicate_variant(error: &CompileError) -> bool {
    matches!(error, CompileError::DuplicateVariant { .. })
}

pub(super) fn is_duplicate_declaration(error: &CompileError) -> bool {
    matches!(error, CompileError::DuplicateDeclaration { .. })
}

pub(super) fn is_missing_selector_annotation(error: &CompileError) -> bool {
    matches!(error, CompileError::MissingSelectorAnnotation { .. })
}

pub(super) fn is_invalid_variant_key(error: &CompileError) -> bool {
    matches!(error, CompileError::InvalidVariantKey { .. })
}

pub(super) fn is_bad_option(error: &FormatError) -> bool {
    matches!(
        error,
        FormatError::Function(MessageFunctionError::BadOption)
    )
}

pub(super) fn is_bad_operand(error: &FormatError) -> bool {
    matches!(
        error,
        FormatError::Function(MessageFunctionError::BadOperand)
    )
}

pub(super) fn is_missing_arg(error: &FormatError) -> bool {
    matches!(error, FormatError::MissingArg(_))
}

pub(super) fn is_unknown_function(error: &FormatError) -> bool {
    matches!(error, FormatError::UnknownFunction { .. })
}

pub(super) fn function_error(error: MessageFunctionError) -> FormatError {
    FormatError::Function(error)
}

pub(super) fn missing_arg(name: &str) -> FormatError {
    FormatError::MissingArg(name.to_string())
}

pub(super) fn bad_selector_with_source(expected: FormatError) -> impl Fn(&FormatError) -> bool {
    move |error| {
        matches!(
            error,
            FormatError::BadSelector { source }
                if source.as_deref() == Some(&expected)
        )
    }
}
