// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::compiler::CompileError;
use message_format::runtime::Value;

// ---------------------------------------------------------------------------
// TR35 §2.1 — Escape sequences (S-52, S-53, S-54, S-55)
// ---------------------------------------------------------------------------

/// S-52 — `\\` in text resolves to `\`.
#[test]
fn text_backslash_escape() {
    assert_format("a\\\\b", &[], "a\\b");
}

/// S-52 — `\\` in a quoted literal resolves to `\`.
#[test]
fn quoted_literal_backslash_escape() {
    assert_format("{|a\\\\b| :string}", &[], "a\\b");
}

/// S-53 — `\|` in a quoted literal resolves to `|`.
#[test]
fn quoted_literal_pipe_escape() {
    assert_format("{|a\\|b| :string}", &[], "a|b");
}

/// S-53 — `\{` in text resolves to `{`.
#[test]
fn text_open_brace_escape() {
    assert_format("a\\{b", &[], "a{b");
}

/// S-53 — `\}` in text resolves to `}`.
#[test]
fn text_close_brace_escape() {
    assert_format("a\\}b", &[], "a}b");
}

/// S-55 — `\{` and `\}` inside a quoted literal are allowed.
#[test]
fn quoted_literal_brace_escapes_allowed() {
    assert_format("{|a\\{b\\}c| :string}", &[], "a{b}c");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Messages (S-3, S-4, S-5, S-6, S-7)
// ---------------------------------------------------------------------------

/// S-3 — An empty string is a valid simple message.
#[test]
fn empty_message_is_valid() {
    assert_compiles("");
    assert_format("", &[], "");
}

/// S-4 — Whitespace at start/end of a simple message is significant.
#[test]
fn simple_message_leading_trailing_whitespace_preserved() {
    assert_format("  hello  ", &[], "  hello  ");
}

/// S-5 — Whitespace at start/end of a complex message is NOT significant.
#[test]
fn complex_message_leading_trailing_whitespace_stripped() {
    assert_format("  {{hello}}  ", &[], "hello");
}

/// S-6 — A simple message MUST NOT start with `.` as first non-whitespace.
#[test]
fn simple_message_leading_dot_is_error() {
    assert_compile_err(".notakeyword foo", is_syntax_error);
}

/// S-7 — A complex message begins with `.` keyword or `{{`.
#[test]
fn complex_message_starts_with_double_brace() {
    assert_format("{{hello}}", &[], "hello");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Text and whitespace (S-16, S-17, S-20)
// ---------------------------------------------------------------------------

/// S-16 — Whitespace in text is preserved literally.
#[test]
fn text_whitespace_preserved() {
    assert_format("hello   world", &[], "hello   world");
}

/// S-16 — Tab characters in text are preserved.
#[test]
fn text_tab_preserved() {
    assert_format("a\tb", &[], "a\tb");
}

/// S-16 — Newlines in text are preserved.
#[test]
fn text_newline_preserved() {
    assert_format("{{a\nb}}", &[], "a\nb");
}

/// S-16 — Plain text without placeholders.
#[test]
fn plain_text_passthrough() {
    assert_format("Hello, world!", &[], "Hello, world!");
}

/// S-15 — Pattern MAY be empty.
#[test]
fn empty_pattern_is_valid() {
    assert_format("{{}}", &[], "");
}

/// S-20 — Arbitrary Unicode code points in text are allowed.
#[test]
fn arbitrary_unicode_in_text() {
    assert_format("こんにちは 🌍", &[], "こんにちは 🌍");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Literals (S-43, S-44, S-46, S-47)
// ---------------------------------------------------------------------------

/// S-46 — Quoted and unquoted literals with same code points are equivalent.
#[test]
fn quoted_unquoted_literal_equivalence() {
    let quoted = format("{|42| :number}", &[]);
    let unquoted = format("{42 :number}", &[]);
    assert_eq!(quoted, unquoted);
}

/// S-43 — Empty quoted literal resolves to empty string.
#[test]
fn empty_quoted_literal() {
    assert_format("{|| :string}", &[], "");
}

/// S-44 — All code points in a literal are preserved.
#[test]
fn literal_code_points_preserved() {
    assert_format("{|こんにちは| :string}", &[], "こんにちは");
}

/// S-43 — Literal MAY include arbitrary Unicode.
#[test]
fn literal_allows_arbitrary_unicode() {
    assert_format("{|🌍🌎🌏| :string}", &[], "🌍🌎🌏");
}

/// S-47 — Number literal uses JSON number syntax.
#[test]
fn number_literal_json_syntax() {
    assert_format("{1.5e2 :number}", &[], "150");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Variables and interpolation
// ---------------------------------------------------------------------------

/// S-31 — Simple variable interpolation.
#[test]
fn simple_variable_interpolation() {
    assert_format(
        "Hello { $name :string }!",
        &[("name", Value::Str("World".into()))],
        "Hello World!",
    );
}

/// S-31 — Multiple variables in one message.
#[test]
fn multiple_variables() {
    assert_format(
        "{ $a :string } and { $b :string }",
        &[("a", Value::Str("X".into())), ("b", Value::Str("Y".into()))],
        "X and Y",
    );
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Names (S-48, S-49, S-50)
// ---------------------------------------------------------------------------

/// S-48 — Variable names can contain underscores.
#[test]
fn variable_name_with_underscore() {
    assert_format(
        "{ $my_var :string }",
        &[("my_var", Value::Str("ok".into()))],
        "ok",
    );
}

/// S-48 — Variable names can contain digits (not leading).
#[test]
fn variable_name_with_digit() {
    assert_format(
        "{ $var1 :string }",
        &[("var1", Value::Str("ok".into()))],
        "ok",
    );
}

/// S-48 — `:` after a variable starts a function annotation, not part of the name.
/// `{ $a:b }` is parsed as variable `$a` with function `:b`.
#[test]
fn colon_in_expression_starts_function() {
    // $a:string is equivalent to $a :string — the colon is the function sigil.
    assert_format("{ $a:string }", &[("a", Value::Str("ok".into()))], "ok");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Expressions (S-31, S-32, S-33)
// ---------------------------------------------------------------------------

/// S-32 — Empty expression `{}` is a syntax error.
#[test]
fn empty_expression_is_error() {
    assert_compile_err("{}", is_invalid_expr);
}

/// S-32 — Expression with only whitespace is a syntax error.
#[test]
fn whitespace_only_expression_is_error() {
    assert_compile_err("{   }", is_invalid_expr);
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Markup (S-36, S-37, S-38, S-39)
// ---------------------------------------------------------------------------

/// S-37 — markup-open without a corresponding close is valid.
#[test]
fn markup_open_without_close_is_valid() {
    assert_compiles("{#bold}text");
    assert_format("{#bold}text", &[], "text");
}

/// S-38 — markup-close without a corresponding open is valid.
#[test]
fn markup_close_without_open_is_valid() {
    assert_compiles("text{/bold}");
    assert_format("text{/bold}", &[], "text");
}

/// S-39 — Markup in any order is valid (close before open).
#[test]
fn markup_arbitrary_order_is_valid() {
    assert_compiles("{/b}text{#b}");
    assert_format("{/b}text{#b}", &[], "text");
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Attributes (S-40, S-41)
// ---------------------------------------------------------------------------

/// S-40/S-41 — Attributes have no effect on formatted output.
#[test]
fn attribute_has_no_formatting_effect() {
    // With and without attribute should produce the same output.
    let with_attr = format(
        "{ $x :string @locale=en }",
        &[("x", Value::Str("hi".into()))],
    );
    let without_attr = format("{ $x :string }", &[("x", Value::Str("hi".into()))]);
    assert_eq!(with_attr, without_attr);
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Keywords (S-42)
// ---------------------------------------------------------------------------

/// S-42 — Keywords are case-sensitive and must be lowercase.
#[test]
fn keywords_are_case_sensitive() {
    // .INPUT (uppercase) should not be recognized as a keyword.
    assert_compile_err(".INPUT { $x :string }\n{{{ $x }}}", is_syntax_error);
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Options (S-35)
// ---------------------------------------------------------------------------

/// S-35 — Order of options is not significant.
#[test]
fn option_order_not_significant() {
    let a = format(
        "{ $x :number minimumFractionDigits=2 signDisplay=always }",
        &[("x", Value::Int(5))],
    );
    let b = format(
        "{ $x :number signDisplay=always minimumFractionDigits=2 }",
        &[("x", Value::Int(5))],
    );
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// TR35 §2.2 — Matcher (S-25, S-26)
// ---------------------------------------------------------------------------

/// S-25 — At least one selector is required in a matcher.
#[test]
fn matcher_requires_at_least_one_selector() {
    assert_compile_err(".match\n* {{x}}", |e| {
        matches!(
            e,
            CompileError::InvalidExpr { .. } | CompileError::InvalidLine { .. }
        )
    });
}
