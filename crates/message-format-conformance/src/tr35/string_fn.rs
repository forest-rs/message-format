// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::Value;

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
