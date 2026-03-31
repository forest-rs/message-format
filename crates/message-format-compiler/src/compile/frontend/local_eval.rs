// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::bindings::{DeclFunction, LocalValue};
use super::*;
use crate::syntax::literal::parse_number_literal;

pub(super) fn local_function_may_fail_select(function_spec: &FunctionSpec) -> bool {
    match function_spec.name.as_str() {
        "test:format" => true,
        "test:select" => function_spec.options.iter().any(|option| {
            if option.key == "fails"
                && let FunctionOptionValue::Literal(value) = &option.value
            {
                return value == "select";
            }
            if option.key == "decimalPlaces"
                && let FunctionOptionValue::Literal(value) = &option.value
            {
                let Ok(dp) = value.parse::<usize>() else {
                    return true;
                };
                return dp > 3;
            }
            false
        }),
        _ => false,
    }
}

pub(super) fn normalize_local_function_expression(
    expression: DeclFunction,
    input_functions: &BTreeMap<String, DeclFunction>,
) -> DeclFunction {
    let Some((operand, local_name, _local_has_options)) = function_signature(&expression) else {
        return expression;
    };
    let Some(input_expression) = input_functions.get(&operand) else {
        return expression;
    };
    let Some((_input_operand, input_name, input_has_options)) =
        function_signature(input_expression)
    else {
        return expression;
    };
    if local_name == input_name && input_has_options {
        input_expression.clone()
    } else {
        expression
    }
}

pub(super) fn selector_chain_is_unstable(
    name: &str,
    local_functions: &BTreeMap<String, DeclFunction>,
) -> bool {
    let Some(expr) = local_functions.get(name) else {
        return false;
    };
    if function_has_dynamic_select_option(expr) {
        return true;
    }
    let Some((operand, func_name, _has_options)) = function_signature(expr) else {
        return false;
    };
    if func_name != "number" && func_name != "integer" {
        return false;
    }
    local_functions
        .get(&operand)
        .is_some_and(function_has_select_option)
}

pub(super) fn resolve_alias(
    name: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<String, CompileError> {
    let mut name = name.to_owned();
    for _ in 0..8 {
        let Some(next) = aliases.get(&name).cloned() else {
            return Ok(name);
        };
        name = next;
    }
    Err(CompileError::alias_resolution_overflow(name))
}

fn function_signature(function: &DeclFunction) -> Option<(String, String, bool)> {
    let Operand::Var(var) = &function.operand else {
        return None;
    };
    Some((
        canonicalize_identifier(var),
        function.func.name.clone(),
        !function.func.options.is_empty(),
    ))
}

fn function_has_select_option(function: &DeclFunction) -> bool {
    function
        .func
        .options
        .iter()
        .any(|option| option.key == "select")
}

fn function_has_dynamic_select_option(function: &DeclFunction) -> bool {
    function
        .func
        .options
        .iter()
        .any(|option| option.key == "select" && matches!(option.value, FunctionOptionValue::Var(_)))
}

pub(super) fn apply_literal_function(
    mut literal: String,
    function_spec: &FunctionSpec,
) -> LocalValue {
    let options = function_spec_options_map(function_spec);
    match function_spec.name.as_str() {
        "string" => {
            literal = apply_bidi_dir(literal, options.get("u:dir").map(String::as_str));
        }
        "integer" => {
            if let Some(value) = parse_number_literal(&literal) {
                let truncated = if value.is_sign_negative() {
                    value.ceil()
                } else {
                    value.floor()
                };
                literal = format_signed_number(parse_sign_display(&options), truncated);
            }
        }
        "number" => {
            if let Some(value) = parse_number_literal(&literal) {
                if options.get("style").map(String::as_str) == Some("percent") {
                    let mut pct = value * 100.0;
                    if pct == -0.0 {
                        pct = 0.0;
                    }
                    let digits = options
                        .get("minimumFractionDigits")
                        .and_then(|v| v.parse::<usize>().ok());
                    let rendered = if let Some(min) = digits {
                        format!("{pct:.min$}")
                    } else {
                        pct.to_string()
                    };
                    literal = format!("{rendered}%");
                } else if let Some(dp) = options
                    .get("minimumFractionDigits")
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    literal =
                        format_signed_string(parse_sign_display(&options), format!("{value:.dp$}"));
                } else {
                    literal = format_signed_number(parse_sign_display(&options), value);
                }
            }
        }
        "offset" => {
            if let Some(value) = parse_number_literal(&literal) {
                let preserve_plus = literal.starts_with('+');
                let adjusted = apply_offset(value, &options).unwrap_or(value);
                let sign_display = if preserve_plus {
                    SignDisplay::Always
                } else {
                    parse_sign_display(&options)
                };
                literal = format_signed_number(sign_display, adjusted);
            }
        }
        "currency" => {
            if let (Some(code), Some(number)) =
                (options.get("currency"), parse_number_literal(&literal))
            {
                literal = format!("{code} {number}");
            }
        }
        "percent" | "date" | "time" | "datetime" => {}
        "test:select" | "test:function" | "test:format" => {
            if let Some(dp) = options
                .get("decimalPlaces")
                .and_then(|value| value.parse::<usize>().ok())
            {
                if let Some(value) = parse_number_literal(&literal) {
                    literal = format!("{value:.dp$}");
                }
            }
        }
        _ => {
            return LocalValue::UnknownFunction(function_spec.name.clone());
        }
    }

    let kind = if parse_number_literal(&literal).is_some() {
        OperandLiteralKind::Number
    } else {
        OperandLiteralKind::String
    };
    LocalValue::Literal {
        value: literal,
        kind,
    }
}

fn function_spec_options_map(function_spec: &FunctionSpec) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for option in &function_spec.options {
        if let FunctionOptionValue::Literal(value) = &option.value {
            map.insert(strip_bidi_controls(&option.key), strip_bidi_controls(value));
        }
    }
    map
}

#[derive(Clone, Copy)]
enum SignDisplay {
    Auto,
    Always,
    Never,
}

fn parse_sign_display(options: &BTreeMap<String, String>) -> SignDisplay {
    match options.get("signDisplay").map(String::as_str) {
        Some("always") => SignDisplay::Always,
        Some("never") => SignDisplay::Never,
        Some("auto") | None => SignDisplay::Auto,
        Some(other) => unreachable!("unexpected validated signDisplay value: {other}"),
    }
}

fn format_signed_number(sign_display: SignDisplay, value: f64) -> String {
    let out = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    };
    format_signed_string(sign_display, out)
}

fn format_signed_string(sign_display: SignDisplay, value: String) -> String {
    match sign_display {
        SignDisplay::Auto => value,
        SignDisplay::Always => {
            if value.starts_with('-') || value.starts_with('+') {
                value
            } else {
                format!("+{value}")
            }
        }
        SignDisplay::Never => {
            if let Some(stripped) = value.strip_prefix('-').or_else(|| value.strip_prefix('+')) {
                stripped.to_string()
            } else {
                value
            }
        }
    }
}

fn apply_offset(value: f64, options: &BTreeMap<String, String>) -> Option<f64> {
    let add = options.get("add").and_then(|raw| raw.parse::<f64>().ok());
    let subtract = options
        .get("subtract")
        .and_then(|raw| raw.parse::<f64>().ok());
    match (add, subtract) {
        (Some(v), None) => Some(value + v),
        (None, Some(v)) => Some(value - v),
        _ => None,
    }
}

fn apply_bidi_dir(value: String, dir: Option<&str>) -> String {
    let isolate_open = match dir.unwrap_or("auto") {
        "ltr" => '\u{2066}',
        "rtl" => '\u{2067}',
        _ => '\u{2068}',
    };
    format!("{isolate_open}{value}\u{2069}")
}
