// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::{MessageFunctionError, Value};

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
    assert_format_with_error(
        ".input { $x :string }\n.match $x\na {{A}}\n* {{CATCHALL}}",
        &[], // no args → selector can't resolve
        "CATCHALL",
        bad_selector_with_source(missing_arg("x")),
    );
}

/// E-7/SEL-5 — Invalid runtime selector options still emit `Bad Selector`.
#[test]
fn selector_bad_option_falls_to_catchall() {
    assert_format_with_error(
        ".input { $mode }\n.input { $x :number select=$mode }\n.match $x\none {{ONE}}\n* {{CATCHALL}}",
        &[("mode", Value::Str("bogus".into())), ("x", Value::Int(1))],
        "CATCHALL",
        bad_selector_with_source(function_error(MessageFunctionError::BadOption)),
    );
}

/// E-7/SEL-5 — Selector diagnostics keep a primary cause without dropping additional missing args.
#[test]
fn selector_with_multiple_missing_inputs_reports_all_missing_args() {
    let output = format_output(
        ".input { $x :number select=$mode minimumFractionDigits=$digits }\n.match $x\none {{ONE}}\n* {{CATCHALL}}",
        &[],
    );
    assert_eq!(output.value, "CATCHALL");
    assert!(
        output
            .errors
            .iter()
            .any(bad_selector_with_source(missing_arg("x"))),
        "missing primary bad-selector diagnostic: {:?}",
        output.errors
    );
    assert!(
        output.errors.contains(&missing_arg("mode")),
        "missing secondary missing-arg diagnostic: {:?}",
        output.errors
    );
    assert!(
        output.errors.contains(&missing_arg("digits")),
        "missing tertiary missing-arg diagnostic: {:?}",
        output.errors
    );
}
