// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::{FormatError, MessageFunctionError, Value};

// ---------------------------------------------------------------------------
// TR35 §12 — Pattern selection basics (SEL-2, SEL-11)
// ---------------------------------------------------------------------------

/// SEL-2 — Message without matcher returns its single pattern.
#[test]
fn no_matcher_returns_single_pattern() {
    assert_format("Hello!", &[], "Hello!");
}

/// SEL-11 — Catch-all `*` always matches.
#[test]
fn catchall_matches() {
    assert_format(
        ".input { $x :string }\n.match $x\n* {{fallback}}",
        &[("x", Value::Str("anything".into()))],
        "fallback",
    );
}

// ---------------------------------------------------------------------------
// TR35 §12 — Exact vs catch-all (SEL-13)
// ---------------------------------------------------------------------------

/// SEL-13 — Exact key match is preferred over catch-all.
#[test]
fn exact_match_preferred_over_catchall() {
    assert_format(
        ".input { $x :string }\n.match $x\na {{MATCH}}\n* {{fallback}}",
        &[("x", Value::Str("a".into()))],
        "MATCH",
    );
}

/// SEL-13 — When no exact match, catch-all is used.
#[test]
fn no_exact_match_uses_catchall() {
    assert_format(
        ".input { $x :string }\n.match $x\na {{A}}\n* {{OTHER}}",
        &[("x", Value::Str("b".into()))],
        "OTHER",
    );
}

// ---------------------------------------------------------------------------
// TR35 §12 — Multi-selector (SEL-3, SEL-4, SEL-12)
// ---------------------------------------------------------------------------

/// SEL-3 — Multi-selector: both keys must match.
#[test]
fn multi_selector_both_match() {
    assert_format(
        ".input { $a :string }\n.input { $b :string }\n.match $a $b\nx y {{BOTH}}\n* * {{OTHER}}",
        &[("a", Value::Str("x".into())), ("b", Value::Str("y".into()))],
        "BOTH",
    );
}

/// SEL-3 — Multi-selector: partial match falls through to catch-all.
#[test]
fn multi_selector_partial_match_falls_through() {
    assert_format(
        ".input { $a :string }\n.input { $b :string }\n.match $a $b\nx y {{BOTH}}\n* * {{OTHER}}",
        &[("a", Value::Str("x".into())), ("b", Value::Str("z".into()))],
        "OTHER",
    );
}

/// SEL-12 — Variant with wildcard in one position.
#[test]
fn wildcard_in_one_selector_position() {
    assert_format(
        ".input { $a :string }\n.input { $b :string }\n.match $a $b\nx * {{X-ANY}}\n* * {{OTHER}}",
        &[
            ("a", Value::Str("x".into())),
            ("b", Value::Str("whatever".into())),
        ],
        "X-ANY",
    );
}

/// SEL-12 — Earlier selectors have higher priority; exact on first wins.
#[test]
fn first_selector_exact_preferred() {
    assert_format(
        ".input { $a :string }\n.input { $b :string }\n.match $a $b\nx * {{X-ANY}}\n* y {{ANY-Y}}\n* * {{OTHER}}",
        &[("a", Value::Str("x".into())), ("b", Value::Str("y".into()))],
        "X-ANY",
    );
}

// ---------------------------------------------------------------------------
// TR35 §12 — Variant ordering and uniqueness (S-24, SEL-14, SEL-15)
// ---------------------------------------------------------------------------

/// S-24 — Duplicate variant keys are a compile error.
#[test]
fn duplicate_variant_key_is_error() {
    assert_compile_err(
        ".input { $x :string }\n.match $x\na {{FIRST}}\na {{SECOND}}\n* {{OTHER}}",
        is_duplicate_variant,
    );
}

/// SEL-15 — Multiple variant keys: correct one selected.
#[test]
fn string_selection_multiple_variants() {
    assert_format(
        ".input { $x :string }\n.match $x\na {{A}}\nb {{B}}\nc {{C}}\n* {{OTHER}}",
        &[("x", Value::Str("b".into()))],
        "B",
    );
}

// ---------------------------------------------------------------------------
// TR35 §12 — Key comparison (S-28, S-29, S-30)
// ---------------------------------------------------------------------------

/// S-30 — Literal keys compared by contents, not syntax (quoted vs unquoted).
#[test]
fn literal_key_quoted_unquoted_equal() {
    // `|a|` and `a` as keys should be equivalent — duplicate detected.
    assert_compile_err(
        ".input { $x :string }\n.match $x\na {{A}}\n|a| {{QUOTED-A}}\n* {{OTHER}}",
        is_duplicate_variant,
    );
}

// ---------------------------------------------------------------------------
// TR35 §12 — Selection with errors (E-7, SEL-5)
// ---------------------------------------------------------------------------

/// E-7/SEL-5 — Selector with missing arg: only catch-all matches.
#[test]
fn selector_missing_arg_falls_to_catchall() {
    let output = format_output(
        ".input { $x :string }\n.match $x\na {{A}}\n* {{CATCHALL}}",
        &[], // no args → selector can't resolve
    );
    assert_eq!(output.value, "CATCHALL");
    assert_errors_multiset(
        &output.errors,
        &[
            FormatError::MissingArg("x".into()),
            FormatError::BadSelector { source: None },
        ],
    );
}

/// E-7/SEL-5 — Invalid runtime selector options still emit `Bad Selector`.
#[test]
fn selector_bad_option_falls_to_catchall() {
    let output = format_output(
        ".input { $mode }\n.input { $x :number select=$mode }\n.match $x\none {{ONE}}\n* {{CATCHALL}}",
        &[("mode", Value::Str("bogus".into())), ("x", Value::Int(1))],
    );
    assert_eq!(output.value, "CATCHALL");
    assert_errors_multiset(
        &output.errors,
        &[
            function_error(MessageFunctionError::BadOption),
            FormatError::BadSelector { source: None },
        ],
    );
}

/// E-7/SEL-5 — Operand failure short-circuits option resolution for the selector.
#[test]
fn selector_operand_failure_short_circuits_option_resolution() {
    let output = format_output(
        ".input { $x :number select=$mode minimumFractionDigits=$digits }\n.match $x\none {{ONE}}\n* {{CATCHALL}}",
        &[],
    );
    assert_eq!(output.value, "CATCHALL");
    assert_errors_multiset(
        &output.errors,
        &[missing_arg("x"), FormatError::BadSelector { source: None }],
    );
}

/// A fallback propagated through a local is re-annotated using the current
/// variable's fallback representation, without invoking the function host.
#[test]
fn local_fallback_reannotation_uses_current_variable_name() {
    let output = format_output(
        ".local $a = {$missing}\n.local $b = {$a :number}\n{{{$b}}}",
        &[],
    );
    assert_eq!(output.value, "{$b}");
    assert_errors_multiset(&output.errors, &[missing_arg("missing")]);
}

/// Plain local aliases retain their own fallback identity when rendered.
#[test]
fn local_fallback_aliases_use_each_variable_name() {
    let output = format_output(
        ".local $a = {$missing}\n.local $b = {$a}\n{{{$a} {$b}}}",
        &[],
    );
    assert_eq!(output.value, "{$a} {$b}");
    assert_errors_multiset(&output.errors, &[missing_arg("missing")]);
}
