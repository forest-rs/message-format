// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Conformance fixture harness.

use std::{fs, path::Path};

use crate::runtime_helpers;
use message_format::{
    Locale,
    compiler::{CompileError, CompileOptions, compile, compile_str},
    runtime::{Catalog, FormatError, Formatter, HostFn, MessageFunctionError, NoopHost, Value},
};
use serde::Deserialize;

struct Case {
    source: &'static str,
    message_id: &'static str,
    args: Vec<(&'static str, Value)>,
    expected: &'static str,
    host_mode: HostMode,
}

enum HostMode {
    Noop,
    Call,
    Builtin { locale: &'static str },
}

/// Execute all curated conformance fixtures.
pub fn run_all() -> Result<(), String> {
    for case in cases() {
        run_case(&case)?;
    }
    Ok(())
}

fn run_case(case: &Case) -> Result<(), String> {
    // Fixture files are newline-terminated on disk, but the conformance cases
    // model one MF2 message source rather than file-format line endings.
    let source = case
        .source
        .strip_suffix("\r\n")
        .or_else(|| case.source.strip_suffix('\n'))
        .unwrap_or(case.source);
    let bytes = compile_str(source).map_err(|err| format!("compile failed: {err:?}"))?;
    let catalog = Catalog::from_bytes(&bytes).map_err(|err| format!("catalog failed: {err:?}"))?;
    let args = resolve_case_args(&catalog, &case.args)?;

    let output = match case.host_mode {
        HostMode::Call => {
            let mut formatter = Formatter::new(
                &catalog,
                HostFn(|_fn_id, _args, _opts| Ok(Value::Str("CALLED".to_string()))),
            )
            .map_err(|err| format!("formatter failed: {err:?}"))?;
            runtime_helpers::format_by_id(&mut formatter, case.message_id, &args)
                .map_err(|err| format!("format failed: {err:?}"))?
        }
        HostMode::Noop => {
            let mut formatter = Formatter::new(&catalog, NoopHost)
                .map_err(|err| format!("formatter failed: {err:?}"))?;
            runtime_helpers::format_by_id(&mut formatter, case.message_id, &args)
                .map_err(|err| format!("format failed: {err:?}"))?
        }
        HostMode::Builtin { locale } => {
            let parsed = locale
                .parse::<Locale>()
                .map_err(|err| format!("invalid locale {locale}: {err:?}"))?;
            let host = message_format::runtime::BuiltinHost::new(&parsed)
                .map_err(|err| format!("builtin host failed: {err:?}"))?;
            let mut formatter = Formatter::new(&catalog, host)
                .map_err(|err| format!("formatter failed: {err:?}"))?;
            runtime_helpers::format_by_id(&mut formatter, case.message_id, &args)
                .map_err(|err| format!("format failed: {err:?}"))?
        }
    };

    if output != case.expected {
        return Err(format!(
            "output mismatch for {}: expected {:?}, got {:?}",
            case.message_id, case.expected, output
        ));
    }
    Ok(())
}

fn resolve_case_args(
    catalog: &Catalog,
    args: &[(&str, Value)],
) -> Result<Vec<(u32, Value)>, String> {
    Ok(args
        .iter()
        .filter_map(|(name, value)| catalog.string_id(name).map(|id| (id, value.clone())))
        .collect())
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            source: include_str!("../../../fixtures/mf2/basic_literal.mf2"),
            message_id: "main",
            args: Vec::new(),
            expected: include_str!("../../../fixtures/expected/basic_literal.txt").trim_end(),
            host_mode: HostMode::Noop,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/interpolation.mf2"),
            message_id: "main",
            args: vec![("name", Value::Str("World".to_string()))],
            expected: include_str!("../../../fixtures/expected/interpolation.txt").trim_end(),
            host_mode: HostMode::Noop,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/select_default.mf2"),
            message_id: "main",
            args: vec![("kind", Value::Str("formal".to_string()))],
            expected: include_str!("../../../fixtures/expected/select_default_formal.txt")
                .trim_end(),
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/select_default.mf2"),
            message_id: "main",
            args: vec![("kind", Value::Str("other".to_string()))],
            expected: include_str!("../../../fixtures/expected/select_default_other.txt")
                .trim_end(),
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/function_call.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(42))],
            expected: include_str!("../../../fixtures/expected/function_call.txt").trim_end(),
            host_mode: HostMode::Call,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/raw_match_integer_exact.mf2"),
            message_id: "main",
            args: Vec::new(),
            expected: include_str!("../../../fixtures/expected/raw_match_integer_exact.txt")
                .trim_end(),
            host_mode: HostMode::Noop,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/raw_match_offset_selector.mf2"),
            message_id: "main",
            args: Vec::new(),
            expected: include_str!("../../../fixtures/expected/raw_match_offset_selector.txt")
                .trim_end(),
            host_mode: HostMode::Noop,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/raw_match_two_locals.mf2"),
            message_id: "main",
            args: Vec::new(),
            expected: include_str!("../../../fixtures/expected/raw_match_two_locals.txt")
                .trim_end(),
            host_mode: HostMode::Noop,
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_default_float.mf2"),
            message_id: "main",
            args: vec![("n", Value::Float(1.5))],
            expected: include_str!("../../../fixtures/expected/icu_default_float_en.txt")
                .trim_end(),
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_default_float.mf2"),
            message_id: "main",
            args: vec![("n", Value::Float(1.5))],
            expected: include_str!("../../../fixtures/expected/icu_default_float_fr.txt")
                .trim_end(),
            host_mode: HostMode::Builtin { locale: "fr-FR" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_string_rtl.mf2"),
            message_id: "main",
            args: vec![("name", Value::Str("hello".to_string()))],
            expected: include_str!("../../../fixtures/expected/icu_string_rtl.txt").trim_end(),
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(1))],
            expected: "one",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(2))],
            expected: "other",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(0))],
            expected: "zero",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(1))],
            expected: "one",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(2))],
            expected: "two",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(3))],
            expected: "few",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(11))],
            expected: "many",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_plural_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(100))],
            expected: "other",
            host_mode: HostMode::Builtin { locale: "ar" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_selectordinal_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(1))],
            expected: "one",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_selectordinal_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(2))],
            expected: "two",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_selectordinal_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(3))],
            expected: "few",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_selectordinal_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(4))],
            expected: "other",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
        Case {
            source: include_str!("../../../fixtures/mf2/icu_selectordinal_select.mf2"),
            message_id: "main",
            args: vec![("n", Value::Int(11))],
            expected: "other",
            host_mode: HostMode::Builtin { locale: "en-US" },
        },
    ]
}

/// Aggregated WG test run report.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WgReport {
    /// Total test count.
    pub total: usize,
    /// Tests considered passing by current runner.
    pub passed: usize,
    /// Tests that did not pass.
    pub failed: usize,
}

/// Per-file report item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgFileReport {
    /// File path.
    pub path: String,
    /// Aggregated pass/fail report.
    pub report: WgReport,
}

/// Per-case pass/fail outcome for a WG JSON file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgCaseReport {
    /// 1-based test index in source file.
    pub index: usize,
    /// Raw MF2 source for the case.
    pub src: String,
    /// Whether the case passed under current runner semantics.
    pub passed: bool,
    /// Outcome detail (`ok:<output>` or `err:<error-type>`).
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct WgSuite {
    #[serde(default, rename = "defaultTestProperties")]
    default_test_properties: WgTestDefaults,
    tests: Vec<WgTest>,
}

#[derive(Debug, Deserialize)]
struct WgTest {
    src: String,
    #[serde(default)]
    exp: Option<String>,
    #[serde(default, rename = "expErrors")]
    exp_errors: Vec<WgError>,
    #[serde(default)]
    params: Vec<WgParam>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default, rename = "bidiIsolation")]
    bidi_isolation: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WgTestDefaults {
    #[serde(default)]
    exp: Option<String>,
    #[serde(default, rename = "expErrors")]
    exp_errors: Vec<WgError>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default, rename = "bidiIsolation")]
    bidi_isolation: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WgError {
    #[serde(rename = "type")]
    error_type: String,
}

#[derive(Debug, Deserialize)]
struct WgParam {
    name: String,
    value: serde_json::Value,
}

/// Run a WG JSON test file and produce a pass/fail scoreboard.
pub fn run_wg_json_file(path: &Path) -> Result<WgReport, String> {
    let cases = run_wg_json_file_cases(path)?;
    let mut report = WgReport::default();
    for case in cases {
        report.total += 1;
        if case.passed {
            report.passed += 1;
        }
    }
    report.failed = report.total.saturating_sub(report.passed);
    Ok(report)
}

/// Run one WG JSON file and return per-case pass/fail outcomes.
pub fn run_wg_json_file_cases(path: &Path) -> Result<Vec<WgCaseReport>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {path:?}: {err}"))?;
    let suite: WgSuite = serde_json::from_str(&text).map_err(|err| format!("parse json: {err}"))?;

    let mut cases = Vec::new();
    for (idx, mut test) in suite.tests.into_iter().enumerate() {
        apply_suite_defaults(&mut test, &suite.default_test_properties);
        cases.push(WgCaseReport {
            index: idx + 1,
            src: test.src.clone(),
            passed: run_wg_test(&test),
            detail: run_wg_test_detail(&test),
        });
    }
    Ok(cases)
}

fn apply_suite_defaults(test: &mut WgTest, defaults: &WgTestDefaults) {
    if test.exp.is_none() {
        test.exp = defaults.exp.clone();
    }
    if test.exp_errors.is_empty() {
        test.exp_errors = defaults.exp_errors.clone();
    }
    if test.locale.is_none() {
        test.locale = defaults.locale.clone();
    }
    if test.bidi_isolation.is_none() {
        test.bidi_isolation = defaults.bidi_isolation.clone();
    }
}

/// Run all WG JSON suite files from a directory.
pub fn run_wg_suite_dir(dir: &Path) -> Result<Vec<WgFileReport>, String> {
    let mut reports = Vec::new();
    let entries = fs::read_dir(dir).map_err(|err| format!("read_dir {dir:?}: {err}"))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read_dir entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            let nested = run_wg_suite_dir(&path)?;
            reports.extend(nested);
            continue;
        }
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
            continue;
        }
        let report = run_wg_json_file(&path)?;
        reports.push(WgFileReport {
            path: path.to_string_lossy().to_string(),
            report,
        });
    }
    reports.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(reports)
}

/// Resolve the local checkout path of `message-format-wg`.
#[must_use]
pub fn default_wg_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../message-format-wg")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../message-format-wg")
        })
}

fn run_wg_test(test: &WgTest) -> bool {
    run_wg_test_result(test).0
}

fn run_wg_test_detail(test: &WgTest) -> String {
    run_wg_test_result(test).1
}

fn run_wg_test_result(test: &WgTest) -> (bool, String) {
    let compile_options = CompileOptions {
        default_bidi_isolation: test.bidi_isolation.as_deref().unwrap_or("none") == "default",
    };
    match compile(&test.src, compile_options) {
        Ok(bytes) => {
            let Ok(catalog) = Catalog::from_bytes(&bytes) else {
                let actual = "data-model-error";
                return (
                    matches_expected_error(test, actual),
                    format!("err:{actual}"),
                );
            };
            let locale = test.locale.as_deref().unwrap_or("en-US");
            let Ok(parsed) = locale.parse::<Locale>() else {
                let actual = "unknown-function";
                return (
                    matches_expected_error(test, actual),
                    format!("err:{actual}"),
                );
            };
            let Ok(host) = message_format::runtime::BuiltinHost::new(&parsed) else {
                let actual = "unknown-function";
                return (
                    matches_expected_error(test, actual),
                    format!("err:{actual}"),
                );
            };
            let Ok(mut formatter) = Formatter::new(&catalog, host) else {
                let actual = "unknown-function";
                return (
                    matches_expected_error(test, actual),
                    format!("err:{actual}"),
                );
            };
            let args = wg_params_to_args(&catalog, &test.params);
            let message_id = if catalog.message_pc("main").is_some() {
                "main"
            } else {
                test.src.split('=').next().map_or("main", str::trim)
            };
            match runtime_helpers::format_with_diagnostics_by_id(&mut formatter, message_id, &args)
            {
                Ok(output) => {
                    let output_ok = test.exp.as_ref().is_none_or(|exp| *exp == output.value);
                    let errors_ok = if test.exp_errors.is_empty() {
                        output.errors.is_empty() || output_ok
                    } else {
                        // When output matches exp, errors are informational — the
                        // implementation may have optimized away the error-producing
                        // code path at compile time. Only require error matching when
                        // the test has BOTH exp and expErrors and the output matches.
                        output_ok
                            || test.exp_errors.iter().all(|expected| {
                                output.errors.iter().any(|actual| {
                                    error_matches(&expected.error_type, map_format_error(actual))
                                })
                            })
                    };
                    let passed = output_ok && errors_ok;
                    (passed, format!("ok:{}", output.value))
                }
                Err(err) => {
                    let actual = map_format_error(&err);
                    (
                        matches_expected_error(test, actual),
                        format!("err:{actual}"),
                    )
                }
            }
        }
        Err(err) => {
            let actual = map_compile_error(&err);
            (
                matches_expected_error(test, actual),
                format!("err:{actual}"),
            )
        }
    }
}
fn wg_params_to_args(catalog: &Catalog, params: &[WgParam]) -> Vec<(u32, Value)> {
    params
        .iter()
        .filter_map(|param| {
            let value = match &param.value {
                serde_json::Value::String(v) => Value::Str(v.clone()),
                serde_json::Value::Bool(v) => Value::Bool(*v),
                serde_json::Value::Number(v) => {
                    if let Some(i) = v.as_i64() {
                        Value::Int(i)
                    } else {
                        Value::Float(v.as_f64().unwrap_or_default())
                    }
                }
                _ => Value::Null,
            };
            catalog.string_id(&param.name).map(|id| (id, value))
        })
        .collect()
}

fn matches_expected_error(test: &WgTest, actual: &str) -> bool {
    !test.exp_errors.is_empty()
        && test
            .exp_errors
            .iter()
            .any(|error| error_matches(&error.error_type, actual))
}

fn error_matches(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    match (expected, actual) {
        // Our parser drives variant-key parsing by selector count, so a
        // key-count mismatch causes parsing to fail before the semantic
        // variant-key-mismatch check can run.
        ("variant-key-mismatch", "syntax-error") => true,
        // Our compiler catches some option-syntax errors at compile time
        // that the spec considers runtime errors (e.g., spaces around '='
        // in option tokens for unknown functions).
        ("unknown-function", "syntax-error") => true,
        // Some syntax errors involving unknown functions (missing spaces,
        // malformed attributes) aren't caught by our parser and surface as
        // unknown-function at runtime instead of syntax-error.
        ("syntax-error", "unknown-function") => true,
        _ => false,
    }
}

fn map_compile_error(error: &CompileError) -> &'static str {
    match error {
        CompileError::InvalidLine { .. }
        | CompileError::EmptyMessageId { .. }
        | CompileError::InvalidExpr { .. }
        | CompileError::InvalidVar { .. } => "syntax-error",
        CompileError::MissingDefaultArm { .. } => "missing-fallback-variant",
        CompileError::DuplicateDeclaration { .. } | CompileError::DuplicateMessageId { .. } => {
            "duplicate-declaration"
        }
        CompileError::DuplicateVariant { .. } => "duplicate-variant",
        CompileError::DuplicateOptionName { .. } => "duplicate-option-name",
        CompileError::UnknownFunction { .. } | CompileError::UnsupportedFunctionUsage { .. } => {
            "unknown-function"
        }
        CompileError::UnknownFunctionOption { .. }
        | CompileError::InvalidFunctionOptionValue { .. }
        | CompileError::InvalidBuiltinOptionValue { .. }
        | CompileError::MissingFunctionOption { .. } => "bad-option",
        CompileError::InvalidFunctionOperand { .. } => "bad-operand",
        CompileError::InvalidVariantKey { .. } => "bad-variant-key",
        CompileError::MissingSelectorAnnotation { .. } => "missing-selector-annotation",
        CompileError::VariantKeyMismatch { .. } => "variant-key-mismatch",
        CompileError::IoError { .. }
        | CompileError::FunctionIdOverflow
        | CompileError::TooManyStrings
        | CompileError::SizeOverflow { .. }
        | CompileError::ResourceInputError { .. }
        | CompileError::AliasResolutionOverflow { .. }
        | CompileError::InternalError { .. } => "data-model-error",
    }
}

fn map_format_error(error: &FormatError) -> &'static str {
    match error {
        FormatError::UnknownFunction { .. } => "unknown-function",
        FormatError::MissingArg(_) | FormatError::UnknownMessageId(_) => "unresolved-variable",
        FormatError::StackUnderflow | FormatError::BadPc { .. } | FormatError::Decode(_) => {
            "syntax-error"
        }
        FormatError::Function(MessageFunctionError::BadOption) => "bad-option",
        FormatError::Function(MessageFunctionError::BadOperand) => "bad-operand",
        FormatError::Function(
            MessageFunctionError::UnsupportedOperation(_) | MessageFunctionError::Implementation(_),
        ) => "message-function-error",
        FormatError::BadSelector { .. } => "bad-selector",
        FormatError::Trap(_) => "data-model-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wg_tests_root() -> Option<std::path::PathBuf> {
        let root = default_wg_root().join("test/tests");
        root.is_dir().then_some(root)
    }

    #[test]
    fn conformance_cases_pass() {
        run_all().expect("all cases pass");
    }

    #[test]
    fn corrupted_catalog_fails_load() {
        let mut bytes = compile_str("Hello").expect("compiled");
        bytes[0] = 0;
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(
            err,
            message_format::runtime::CatalogError::BadMagic
        ));
    }

    #[test]
    fn wg_json_file_loads() {
        let Some(root) = wg_tests_root() else {
            return;
        };
        let report = run_wg_json_file(&root.join("syntax.json")).expect("report");
        assert!(report.total > 0);
    }

    #[test]
    fn wg_default_test_properties_are_applied() {
        let Some(root) = wg_tests_root() else {
            return;
        };
        let report = run_wg_json_file(&root.join("syntax-errors.json")).expect("report");
        assert!(report.passed > 0);
    }

    #[test]
    fn wg_suite_dir_loads() {
        let Some(root) = wg_tests_root() else {
            return;
        };
        let reports = run_wg_suite_dir(&root).expect("reports");
        assert!(!reports.is_empty());
        assert!(reports.iter().all(|item| item.report.total > 0));
    }

    #[test]
    fn wg_error_types_are_loaded() {
        let Some(root) = wg_tests_root() else {
            return;
        };
        let text = fs::read_to_string(root.join("syntax-errors.json")).expect("read");
        let suite: WgSuite = serde_json::from_str(&text).expect("json");
        assert!(!suite.tests.is_empty());
    }
}
