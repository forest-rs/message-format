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
    /// Outcome detail, including mapped diagnostics for formatted cases.
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct WgSuite {
    #[serde(default, rename = "defaultTestProperties")]
    default_test_properties: WgTestDefaults,
    tests: Vec<WgTest>,
}

#[derive(Debug, Default, Deserialize)]
struct WgTest {
    src: String,
    #[serde(default)]
    exp: Option<String>,
    #[serde(default, rename = "expParts")]
    exp_parts: Option<serde_json::Value>,
    #[serde(default, rename = "expErrors")]
    exp_errors: Option<Vec<WgError>>,
    #[serde(default)]
    params: Option<Vec<WgParam>>,
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
    exp_errors: Option<Vec<WgError>>,
    #[serde(default, rename = "expParts")]
    exp_parts: Option<serde_json::Value>,
    #[serde(default)]
    params: Option<Vec<WgParam>>,
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

#[derive(Debug, Clone, Deserialize)]
struct WgParam {
    name: String,
    value: serde_json::Value,
    #[serde(default, rename = "type")]
    value_type: Option<String>,
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
        let (passed, detail) = run_wg_test_result(&test);
        cases.push(WgCaseReport {
            index: idx + 1,
            src: test.src.clone(),
            passed,
            detail,
        });
    }
    Ok(cases)
}

fn apply_suite_defaults(test: &mut WgTest, defaults: &WgTestDefaults) {
    if test.exp.is_none() {
        test.exp = defaults.exp.clone();
    }
    if test.exp_errors.is_none() {
        test.exp_errors = defaults.exp_errors.clone();
    }
    if test.exp_parts.is_none() {
        test.exp_parts = defaults.exp_parts.clone();
    }
    if test.params.is_none() {
        test.params = defaults.params.clone();
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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .map(|ancestor| ancestor.join("message-format-wg"))
        .find(|candidate| candidate.is_dir())
        .unwrap_or_else(|| manifest_dir.join("../../../message-format-wg"))
}

fn run_wg_test_result(test: &WgTest) -> (bool, String) {
    if test.exp_parts.is_some() {
        return (false, "unsupported-expectation:expParts".to_string());
    }
    let compile_options = CompileOptions {
        default_bidi_isolation: test.bidi_isolation.as_deref().unwrap_or("none") == "default",
        ..CompileOptions::default()
    };
    match compile(&test.src, compile_options) {
        Ok(bytes) => {
            let catalog = match Catalog::from_bytes(&bytes) {
                Ok(catalog) => catalog,
                Err(error) => {
                    return (false, format!("init-error:catalog:{error:?}"));
                }
            };
            let locale = test.locale.as_deref().unwrap_or("en-US");
            let parsed = match locale.parse::<Locale>() {
                Ok(parsed) => parsed,
                Err(_) => {
                    return (false, format!("init-error:locale:{locale}"));
                }
            };
            let host = match message_format::runtime::BuiltinHost::new(&parsed) {
                Ok(host) => host,
                Err(error) => {
                    return (false, format!("init-error:builtin-host:{error:?}"));
                }
            };
            let mut formatter = match Formatter::new(&catalog, host) {
                Ok(formatter) => formatter,
                Err(error) => {
                    return (false, format!("init-error:formatter:{error:?}"));
                }
            };
            let args = match wg_params_to_args(&catalog, test.params.as_deref().unwrap_or(&[])) {
                Ok(args) => args,
                Err(detail) => return (false, format!("unsupported-input:{detail}")),
            };
            let message_id = if catalog.message_pc("main").is_some() {
                "main"
            } else {
                test.src.split('=').next().map_or("main", str::trim)
            };
            match runtime_helpers::format_with_diagnostics_by_id(&mut formatter, message_id, &args)
            {
                Ok(output) => {
                    let actual = output
                        .errors
                        .iter()
                        .flat_map(map_format_errors)
                        .collect::<Vec<_>>();
                    finish_wg_test(test, Some(&output.value), &actual, "ok".to_string())
                }
                Err(err) => {
                    let actual = map_format_errors(&err);
                    finish_wg_test(test, None, &actual, format!("err:{actual:?}"))
                }
            }
        }
        Err(err) => {
            let actual = [map_compile_error(&err)];
            finish_wg_test(test, None, &actual, format!("err:{actual:?}"))
        }
    }
}

fn finish_wg_test(
    test: &WgTest,
    output: Option<&str>,
    actual_errors: &[&'static str],
    detail: String,
) -> (bool, String) {
    let output_ok = test
        .exp
        .as_deref()
        .is_none_or(|expected| Some(expected) == output);
    let errors_ok = expected_errors_match(test.exp_errors.as_deref(), actual_errors);
    let expected = test
        .exp_errors
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|error| error.error_type.as_str())
        .collect::<Vec<_>>();
    (
        output_ok && errors_ok,
        format!(
            "{detail} output={output:?} actual_errors={actual_errors:?} expected_errors={expected:?}"
        ),
    )
}
fn wg_params_to_args(catalog: &Catalog, params: &[WgParam]) -> Result<Vec<(u32, Value)>, String> {
    params
        .iter()
        .filter_map(|param| {
            if let Some(value_type) = &param.value_type {
                return Some(Err(format!(
                    "parameter {:?} has unsupported type {value_type:?}",
                    param.name
                )));
            }
            let value = match &param.value {
                serde_json::Value::String(v) => Ok(Value::Str(v.clone())),
                serde_json::Value::Bool(v) => Ok(Value::Bool(*v)),
                serde_json::Value::Null => Ok(Value::Null),
                serde_json::Value::Number(v) => Ok(if let Some(i) = v.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Float(v.as_f64().unwrap_or_default())
                }),
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => Err(format!(
                    "parameter {:?} has unsupported structured value",
                    param.name
                )),
            };
            // Validate the value before filtering unknown names. Unsupported
            // WG values remain invalid even when the message does not declare
            // a parameter with that name.
            match value {
                Ok(value) => catalog.string_id(&param.name).map(|id| Ok((id, value))),
                Err(error) => Some(Err(error)),
            }
        })
        .collect::<Result<Vec<_>, _>>()
}

fn expected_errors_match(expected: Option<&[WgError]>, actual: &[&'static str]) -> bool {
    let mut expected_counts = std::collections::BTreeMap::new();
    for error in expected.unwrap_or_default() {
        *expected_counts
            .entry(error.error_type.as_str())
            .or_insert(0) += 1;
    }
    let mut actual_counts = std::collections::BTreeMap::new();
    for error in actual {
        *actual_counts.entry(*error).or_insert(0) += 1;
    }
    expected_counts == actual_counts
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

fn map_format_errors(error: &FormatError) -> Vec<&'static str> {
    let mut mapped = Vec::new();
    map_format_error_chain(error, &mut mapped);
    mapped
}

fn map_format_error_chain(error: &FormatError, mapped: &mut Vec<&'static str>) {
    match error {
        FormatError::UnknownFunction { .. } => mapped.push("unknown-function"),
        FormatError::MissingArg(_) => mapped.push("unresolved-variable"),
        FormatError::UnknownMessageId(_) => mapped.push("data-model-error"),
        FormatError::StackUnderflow
        | FormatError::BadPc { .. }
        | FormatError::Decode(_)
        | FormatError::Trap(_) => mapped.push("data-model-error"),
        FormatError::Function(MessageFunctionError::BadOption) => mapped.push("bad-option"),
        FormatError::Function(MessageFunctionError::BadOperand) => mapped.push("bad-operand"),
        FormatError::Function(MessageFunctionError::UnsupportedOperation(_)) => {
            mapped.push("unsupported-operation");
        }
        FormatError::Function(MessageFunctionError::Implementation(_)) => {
            mapped.push("message-function-error");
        }
        FormatError::BadSelector { source } => {
            mapped.push("bad-selector");
            if let Some(source) = source {
                map_format_error_chain(source, mapped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use message_format::runtime::UnsupportedOperation;

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

    fn test_case(src: &str, exp: Option<&str>, errors: Option<&[&str]>) -> WgTest {
        WgTest {
            src: src.to_string(),
            exp: exp.map(str::to_string),
            exp_errors: errors.map(|errors| {
                errors
                    .iter()
                    .map(|error_type| WgError {
                        error_type: (*error_type).to_string(),
                    })
                    .collect()
            }),
            ..WgTest::default()
        }
    }

    #[test]
    fn matching_output_does_not_hide_unexpected_diagnostic() {
        let test = test_case("hello {$missing}", Some("hello {$missing}"), None);
        assert!(!run_wg_test_result(&test).0);
    }

    #[test]
    fn matching_output_does_not_hide_missing_diagnostic() {
        let test = test_case("hello", Some("hello"), Some(&["bad-option"]));
        assert!(!run_wg_test_result(&test).0);
    }

    #[test]
    fn error_only_case_requires_the_expected_diagnostic() {
        let test = test_case("hello", None, Some(&["bad-option"]));
        assert!(!run_wg_test_result(&test).0);
    }

    #[test]
    fn matching_output_does_not_hide_wrong_diagnostic() {
        let test = test_case(
            "hello {$missing}",
            Some("hello {$missing}"),
            Some(&["unknown-function"]),
        );
        assert!(!run_wg_test_result(&test).0);
    }

    #[test]
    fn matching_output_does_not_hide_extra_diagnostic() {
        let test = test_case(
            "{$x} {:f}",
            Some("{$x} {:f}"),
            Some(&["unresolved-variable"]),
        );
        assert!(!run_wg_test_result(&test).0);
    }

    #[test]
    fn explicit_empty_errors_override_nonempty_default() {
        let mut test = test_case("hello", Some("hello"), Some(&[]));
        let defaults = WgTestDefaults {
            exp_errors: Some(vec![WgError {
                error_type: "unresolved-variable".to_string(),
            }]),
            ..WgTestDefaults::default()
        };
        apply_suite_defaults(&mut test, &defaults);
        assert!(test.exp_errors.as_ref().is_some_and(Vec::is_empty));
        assert!(run_wg_test_result(&test).0);
    }

    #[test]
    fn absent_errors_and_explicit_empty_errors_are_distinct() {
        let absent = test_case("hello", Some("hello"), None);
        let empty = test_case("hello", Some("hello"), Some(&[]));
        let defaults = WgTestDefaults {
            exp_errors: Some(vec![WgError {
                error_type: "unresolved-variable".to_string(),
            }]),
            ..WgTestDefaults::default()
        };
        let mut absent = absent;
        apply_suite_defaults(&mut absent, &defaults);
        assert!(absent.exp_errors.is_some_and(|errors| !errors.is_empty()));
        assert!(empty.exp_errors.is_some_and(|errors| errors.is_empty()));
    }

    #[test]
    fn nested_errors_map_to_their_full_error_chain() {
        let error = FormatError::BadSelector {
            source: Some(Box::new(FormatError::MissingArg("selector".to_string()))),
        };
        assert_eq!(
            map_format_errors(&error),
            vec!["bad-selector", "unresolved-variable"]
        );
    }

    #[test]
    fn error_matching_is_an_exact_multiset() {
        let expected_errors = [
            WgError {
                error_type: "bad-selector".to_string(),
            },
            WgError {
                error_type: "bad-selector".to_string(),
            },
        ];
        let expected = Some(&expected_errors[..]);
        assert!(expected_errors_match(
            expected,
            &["bad-selector", "bad-selector"]
        ));
        assert!(!expected_errors_match(expected, &["bad-selector"]));
        assert!(!expected_errors_match(
            expected,
            &["bad-selector", "bad-selector", "bad-selector"]
        ));
    }

    #[test]
    fn fatal_compile_errors_cannot_satisfy_expected_text() {
        let with_text = test_case("{", Some(""), Some(&["syntax-error"]));
        let without_text = test_case("{", None, Some(&["syntax-error"]));
        assert!(!run_wg_test_result(&with_text).0);
        assert!(run_wg_test_result(&without_text).0);
    }

    #[test]
    fn unsupported_operation_keeps_its_runtime_error_kind() {
        let error = FormatError::Function(MessageFunctionError::UnsupportedOperation(
            UnsupportedOperation::DateFormattingForLocale,
        ));
        assert_eq!(map_format_errors(&error), vec!["unsupported-operation"]);
    }

    #[test]
    fn structured_wg_parameters_are_rejected() {
        let source = compile_str("{$object}").expect("compiled");
        let catalog = Catalog::from_bytes(&source).expect("catalog");
        let params = [WgParam {
            name: "object".to_string(),
            value: serde_json::json!({"key": "value"}),
            value_type: None,
        }];
        assert!(wg_params_to_args(&catalog, &params).is_err());
    }

    #[test]
    fn unused_structured_wg_parameters_are_rejected() {
        let source = compile_str("hello").expect("compiled");
        let catalog = Catalog::from_bytes(&source).expect("catalog");
        let params = [WgParam {
            name: "object".to_string(),
            value: serde_json::json!({"key": "value"}),
            value_type: None,
        }];
        assert!(wg_params_to_args(&catalog, &params).is_err());
    }

    #[test]
    fn null_wg_parameters_use_runtime_null() {
        let source = compile_str("{$null}").expect("compiled");
        let catalog = Catalog::from_bytes(&source).expect("catalog");
        let params = [WgParam {
            name: "null".to_string(),
            value: serde_json::Value::Null,
            value_type: None,
        }];
        let args = wg_params_to_args(&catalog, &params).expect("scalar parameter");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].1, Value::Null);
    }

    #[test]
    fn deserialization_preserves_absent_and_empty_error_properties() {
        let absent: WgTest = serde_json::from_str(r#"{"src":"hello"}"#).expect("json");
        let empty: WgTest =
            serde_json::from_str(r#"{"src":"hello","expErrors":[],"expParts":[],"params":[]}"#)
                .expect("json");
        assert!(absent.exp_errors.is_none());
        assert!(empty.exp_errors.is_some_and(|errors| errors.is_empty()));
        assert!(absent.exp_parts.is_none());
        assert!(empty.exp_parts.is_some());
        assert!(absent.params.is_none());
        assert!(empty.params.is_some_and(|params| params.is_empty()));
    }

    #[test]
    fn suite_defaults_reject_inherited_unsupported_expectations() {
        let defaults = WgTestDefaults {
            exp_parts: Some(serde_json::json!([])),
            ..WgTestDefaults::default()
        };
        let mut test = test_case("hello", Some("hello"), Some(&[]));
        apply_suite_defaults(&mut test, &defaults);
        assert_eq!(
            run_wg_test_result(&test),
            (false, "unsupported-expectation:expParts".to_string())
        );
    }

    #[test]
    fn explicit_empty_params_override_inherited_params() {
        let defaults = WgTestDefaults {
            params: Some(vec![WgParam {
                name: "unused".to_string(),
                value: serde_json::json!({"key": "value"}),
                value_type: None,
            }]),
            ..WgTestDefaults::default()
        };
        let mut test = test_case("hello", Some("hello"), Some(&[]));
        test.params = Some(Vec::new());
        apply_suite_defaults(&mut test, &defaults);
        assert!(run_wg_test_result(&test).0);
    }

    #[test]
    fn absent_params_inherit_and_reject_unsupported_values() {
        let defaults = WgTestDefaults {
            params: Some(vec![WgParam {
                name: "unused".to_string(),
                value: serde_json::json!([1, 2, 3]),
                value_type: None,
            }]),
            ..WgTestDefaults::default()
        };
        let mut test = test_case("hello", Some("hello"), Some(&[]));
        apply_suite_defaults(&mut test, &defaults);
        assert_eq!(
            run_wg_test_result(&test),
            (
                false,
                "unsupported-input:parameter \"unused\" has unsupported structured value"
                    .to_string()
            )
        );
    }

    #[test]
    fn unsupported_parts_expectation_cannot_silently_pass() {
        let test: WgTest =
            serde_json::from_str(r#"{"src":"{#tag}","exp":"","expParts":[{"type":"markup"}]}"#)
                .expect("json");
        let result = run_wg_test_result(&test);
        assert_eq!(
            result,
            (false, "unsupported-expectation:expParts".to_string())
        );
    }
}
