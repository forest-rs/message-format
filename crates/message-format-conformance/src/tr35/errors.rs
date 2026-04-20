// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::compiler::CompileError;
use message_format::runtime::Value;

// ---------------------------------------------------------------------------
// TR35 §6 — Syntax errors (E-9 through E-13)
// ---------------------------------------------------------------------------

/// E-9 — Missing closing brace is a syntax error.
#[test]
fn missing_closing_brace() {
    assert_compile_err("{ $x :string", is_syntax_error);
}

/// E-9 — Malformed declaration is a syntax error.
#[test]
fn malformed_declaration() {
    assert_compile_err(".local = { |x| :string }\n{{}}", is_syntax_error);
}

/// E-9 — Missing opening brace in expression.
#[test]
fn missing_opening_brace() {
    assert_compile_err("$x :string}", is_syntax_error);
}

/// E-13 — Declaration without a message body is a syntax error.
#[test]
fn declaration_without_body() {
    assert_compile_err(".local $var = { |no body| :string }", is_syntax_error);
}

// ---------------------------------------------------------------------------
// TR35 §7 — Data model errors (E-14 through E-25)
// ---------------------------------------------------------------------------

/// E-15 — `.match` without a default `*` variant is a Missing Fallback Variant error.
#[test]
fn missing_fallback_variant() {
    assert_compile_err(
        ".input { $x :string }\n.match $x\na {{A}}",
        is_missing_default_arm,
    );
}

/// E-14 — Variant key count mismatch: two selectors but one key.
#[test]
fn variant_key_count_mismatch() {
    assert_compile_err(
        ".input { $a :string }\n.input { $b :string }\n.match $a $b\nx {{X}}\n* * {{OTHER}}",
        |e| {
            matches!(
                e,
                CompileError::InvalidLine { .. }
                    | CompileError::InvalidExpr { .. }
                    | CompileError::MissingDefaultArm { .. }
                    | CompileError::VariantKeyMismatch { .. }
            )
        },
    );
}

/// E-23 — Duplicate option name is a data model error.
#[test]
fn duplicate_option_name() {
    assert_compile_err(
        "{ $x :number minimumFractionDigits=1 minimumFractionDigits=2 }",
        is_duplicate_option_name,
    );
}

/// A `.match` variant key without a following `{{...}}` pattern is an MF2 syntax error.
#[test]
fn match_dangling_key_without_pattern_is_syntax_error() {
    assert_compile_err(
        ".match $x\none {{One}}\n* {{Other}} .bogus",
        is_syntax_error,
    );
}

/// E-24 — Duplicate variant keys are a compile error.
#[test]
fn duplicate_variant_key() {
    assert_compile_err(
        ".input { $x :string }\n.match $x\na {{FIRST}}\na {{SECOND}}\n* {{OTHER}}",
        is_duplicate_variant,
    );
}

/// E-29 — :number with a non-numeric operand (Bool) is a bad-operand error.
#[test]
fn bad_operand_for_number() {
    assert_format_err(
        "{ $x :number }",
        &[("x", Value::Bool(true))],
        is_bad_operand,
    );
}

// ---------------------------------------------------------------------------
// TR35 §8 — Resolution errors (E-26, E-27)
// ---------------------------------------------------------------------------

/// E-26 — Reference to an undeclared variable is an Unresolved Variable error.
#[test]
fn unresolved_variable() {
    assert_format_err("{ $undefined :string }", &[], is_missing_arg);
}

/// E-27 — Unknown function name is an Unknown Function error.
#[test]
fn unknown_function() {
    assert_format_err("{ |x| :nonexistent }", &[], is_unknown_function);
}

// ---------------------------------------------------------------------------
// TR35 §9 — Function errors (E-29 through E-35)
// ---------------------------------------------------------------------------

/// E-31 — Bad option value for :number.
#[test]
fn bad_option_for_number() {
    assert_format_err(
        "{ $x :number minimumFractionDigits=abc }",
        &[("x", Value::Int(42))],
        is_bad_option,
    );
}

/// E-26 — Missing operand for a function.
#[test]
fn missing_operand_for_function() {
    assert_format_err("{ $x :number }", &[], is_missing_arg);
}

/// E-30 — Literal operand `|horse|` with :number produces Bad Operand.
#[test]
fn bad_operand_literal_for_number() {
    assert_format_err("{ |horse| :number }", &[], is_bad_operand);
}

/// E-7 — Error during selector resolution: only catch-all `*` matches.
#[test]
fn selector_error_falls_back_to_catchall() {
    let source = ".input { $x :string }\n.match $x\na {{A}}\n* {{CATCHALL}}";
    // Missing $x arg → selector resolution error → catch-all
    assert_format(
        source,
        &[], // no args provided
        "CATCHALL",
    );
}

/// E-35 — Bad variant key: key is not valid for the selector function.
#[test]
fn bad_variant_key_non_numeric_for_plural() {
    assert_compile_err(
        ".input { $x :number select=plural }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}",
        is_invalid_variant_key,
    );
}

/// E-35 — Bad variant key: plain numeric selectors reject non-keyword text.
#[test]
fn bad_variant_key_non_numeric_for_default_number_selection() {
    assert_compile_err(
        ".input { $x :number }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}",
        is_invalid_variant_key,
    );
}
