// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::compiler::CompileOptions;
use message_format::runtime::{FormatError, Value};

// ---------------------------------------------------------------------------
// TR35 §13 — :string function
// ---------------------------------------------------------------------------

/// TR35 §13 — :string formats a string operand as-is.
#[test]
fn string_format_passthrough() {
    assert_format(
        "{ $x :string }",
        &[("x", Value::Str("hello".into()))],
        "hello",
    );
}

/// TR35 §13 — :string with a quoted literal operand.
#[test]
fn string_literal_operand() {
    assert_format("{|world| :string}", &[], "world");
}

/// TR35 §13 — :string with an unquoted literal operand.
#[test]
fn string_unquoted_literal_operand() {
    assert_format("{hello :string}", &[], "hello");
}

/// TR35 §13 — :string coerces integer values to their string representation.
#[test]
fn string_coerces_integer() {
    assert_format("{ $x :string }", &[("x", Value::Int(42))], "42");
}

/// TR35 §13 — :string coerces float values to their string representation.
#[test]
fn string_coerces_float() {
    assert_format("{ $x :string }", &[("x", Value::Float(2.75))], "2.75");
}

/// TR35 §13 — :string coerces boolean values.
#[test]
fn string_coerces_bool() {
    assert_format("{ $x :string }", &[("x", Value::Bool(true))], "true");
}

/// TR35 §13 — :string with null produces empty string.
#[test]
fn string_null_is_empty() {
    assert_format("{ $x :string }", &[("x", Value::Null)], "");
}

// ---------------------------------------------------------------------------
// TR35 §13 — :string selection
// ---------------------------------------------------------------------------

/// TR35 §13 — :string selection uses exact code-point matching.
#[test]
fn string_selection_exact_match() {
    assert_format(
        ".input { $x :string }\n.match $x\nhello {{HELLO}}\n* {{OTHER}}",
        &[("x", Value::Str("hello".into()))],
        "HELLO",
    );
}

/// TR35 §13 — :string selection is case-sensitive (no case folding).
#[test]
fn string_selection_case_sensitive() {
    assert_format(
        ".input { $x :string }\n.match $x\nhello {{HELLO}}\n* {{OTHER}}",
        &[("x", Value::Str("Hello".into()))],
        "OTHER",
    );
}

/// Resolved string selection compares raw text without numeric coercion.
#[test]
fn string_selection_does_not_match_numeric_text_canonically() {
    assert_format(
        ".input { $x :string }\n.match $x\n1 {{MATCH}}\n* {{OTHER}}",
        &[("x", Value::Str("1.0".into()))],
        "OTHER",
    );
}

// ---------------------------------------------------------------------------
// TR35 §13 — :string selection: no normalization (F-6)
// ---------------------------------------------------------------------------

/// F-6 — :string selection does NOT perform Unicode Normalization.
#[test]
fn string_selection_no_normalization() {
    // "é" (precomposed U+00E9) vs "é" (e + combining acute U+0065 U+0301)
    // These should NOT match under :string selection.
    assert_format(
        ".input { $x :string }\n.match $x\n|é| {{MATCH}}\n* {{OTHER}}",
        &[("x", Value::Str("e\u{0301}".into()))],
        "OTHER",
    );
}

// ---------------------------------------------------------------------------
// TR35 §13 — :string errors (F-3, F-4)
// ---------------------------------------------------------------------------

/// F-3 — :string with a missing variable is a resolution error.
#[test]
fn string_missing_operand() {
    assert_format_err("{ $x :string }", &[], is_missing_arg);
}

/// A string input is resolved once before a numeric reannotation consumes it.
#[test]
fn string_input_reannotates_to_number() {
    assert_format(
        ".input {$x :string} .local $y = {$x :number} {{value={$y}}}",
        &[("x", Value::Int(1))],
        "value=1",
    );
}

/// A missing string input remains a recoverable missing-argument error.
#[test]
fn missing_string_input_reannotation_errors() {
    let output = format_output(
        ".input {$x :string} .local $y = {$x :number} {{value={$y}}}",
        &[],
    );
    assert_eq!(output.value, "value={$y}");
    assert_errors_multiset(&output.errors, &[FormatError::MissingArg("x".to_string())]);
}

/// Quoted string literals can be consumed by a numeric reannotation.
#[test]
fn string_literal_reannotates_to_number() {
    assert_format(
        ".local $s = {|1| :string} .local $y = {$s :number} {{value={$y}}}",
        &[],
        "value=1",
    );
}

/// Direction metadata is applied when the resolved string is rendered.
#[test]
fn string_explicit_direction_is_applied_at_output() {
    assert_eq!(
        format_with_options_raw(
            "{ $x :string u:dir=rtl }",
            &[("x", Value::Str("hello".into()))],
            CompileOptions {
                default_bidi_isolation: false,
                ..CompileOptions::default()
            },
        ),
        "\u{2067}hello\u{2069}"
    );
}
