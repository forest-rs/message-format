// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::{FormatError, MessageFunctionError, Value};

// ---------------------------------------------------------------------------
// TR35 §14 — :number function
// ---------------------------------------------------------------------------

/// TR35 §14 — :number formats integer operands.
#[test]
fn number_formats_integer() {
    assert_format("{ $x :number }", &[("x", Value::Int(42))], "42");
}

/// TR35 §14 — :number formats float operands.
#[test]
fn number_formats_float() {
    assert_format("{ $x :number }", &[("x", Value::Float(2.75))], "2.75");
}

/// TR35 §14 — :number accepts number-literal strings.
#[test]
fn number_literal_string() {
    assert_format("{42 :number}", &[], "42");
}

/// TR35 §14 — :number formats negative integers.
#[test]
fn number_negative_integer() {
    assert_format("{ $x :number }", &[("x", Value::Int(-5))], "-5");
}

/// TR35 §14 — :number formats negative floats.
#[test]
fn number_negative_float() {
    assert_format("{ $x :number }", &[("x", Value::Float(-2.5))], "-2.5");
}

/// TR35 §14 — :number with minimumFractionDigits pads to minimum.
#[test]
fn number_minimum_fraction_digits() {
    assert_format(
        "{ $x :number minimumFractionDigits=2 }",
        &[("x", Value::Int(5))],
        "5.00",
    );
}

/// TR35 §14 — :number with minimumFractionDigits on a float.
#[test]
fn number_minimum_fraction_digits_float() {
    assert_format(
        "{ $x :number minimumFractionDigits=3 }",
        &[("x", Value::Float(1.5))],
        "1.500",
    );
}

/// TR35 §14 — :number with maximumFractionDigits truncates decimals.
#[test]
fn number_maximum_fraction_digits() {
    assert_format(
        "{ $x :number maximumFractionDigits=1 }",
        &[("x", Value::Float(3.456))],
        "3.5",
    );
}

/// TR35 §14 — :number with Null operand is bad-operand.
#[test]
fn number_null_is_bad_operand() {
    assert_format_err("{ $x :number }", &[("x", Value::Null)], is_bad_operand);
}

// ---------------------------------------------------------------------------
// TR35 §14 — :integer function
// ---------------------------------------------------------------------------

/// TR35 §14 — :integer truncates a float toward zero.
#[test]
fn integer_truncates_float() {
    assert_format("{ $x :integer }", &[("x", Value::Float(3.7))], "3");
}

/// TR35 §14 — :integer truncates negative floats toward zero.
#[test]
fn integer_truncates_negative_float() {
    assert_format("{ $x :integer }", &[("x", Value::Float(-3.7))], "-3");
}

/// TR35 §14 — :integer formats integer operands.
#[test]
fn integer_formats_integer() {
    assert_format("{ $x :integer }", &[("x", Value::Int(42))], "42");
}

/// TR35 §14 — :integer with a literal.
#[test]
fn integer_literal() {
    assert_format("{42 :integer}", &[], "42");
}

/// Resolved numeric locals retain exact integer payloads beyond the f64 safe
/// integer range until the runtime renders them.
#[test]
fn numeric_local_preserves_exact_integer() {
    assert_format(
        ".local $n = {9007199254740993 :number useGrouping=never} {{{$n}}}",
        &[],
        "9007199254740993",
    );
}

/// Re-annotating a resolved number overrides only the supplied options and
/// retains inherited options from the local declaration.
#[test]
fn numeric_local_reannotation_inherits_options() {
    assert_format(
        ".local $n = {4.20 :number minimumFractionDigits=2 signDisplay=always} .local $m = {$n :number minimumFractionDigits=1} {{{$m}}}",
        &[],
        "+4.2",
    );
}

/// Re-annotation of an input number also preserves its resolved options.
#[test]
fn numeric_input_reannotation_inherits_options() {
    assert_format(
        ".input {$n :number minimumFractionDigits=2 signDisplay=always} .local $m = {$n :number minimumFractionDigits=1} {{{$m}}}",
        &[("n", Value::Float(4.2))],
        "+4.2",
    );
}

/// Offset applies to the exact resolved payload while retaining inherited
/// number formatting options.
#[test]
fn numeric_local_offset_inherits_options() {
    assert_format(
        ".local $n = {4.2 :number minimumFractionDigits=2 signDisplay=always} .local $m = {$n :offset add=1} {{{$m}}}",
        &[],
        "+5.20",
    );
}

/// Integer conversion truncates a resolved number and drops inherited digit
/// and fraction formatting options.
#[test]
fn numeric_local_integer_drops_number_options() {
    assert_format(
        ".local $n = {4.2 :number minimumFractionDigits=2 signDisplay=always} .local $m = {$n :integer} {{{$m}}}",
        &[],
        "+4",
    );
}

/// Exact matching uses the numeric payload even when grouped display options
/// are inherited by the resolved local.
#[test]
fn numeric_local_match_uses_exact_payload() {
    assert_format(
        ".local $n = {1234 :number useGrouping=always} .match $n 1234 {{exact}} * {{other}}",
        &[],
        "exact",
    );
}

/// Exact numeric matching takes precedence over the plural keyword arm.
#[test]
fn numeric_local_match_exact_precedes_keyword() {
    assert_format(
        ".local $n = {1 :number minimumFractionDigits=2} .match $n 1 {{exact}} one {{one}} * {{other}}",
        &[],
        "exact",
    );
}

/// A resolved numeric local with no exact arm does not use its display text as
/// a plural category selector.
#[test]
fn numeric_local_match_keyword_uses_plural_category() {
    assert_format(
        ".local $n = {1 :number minimumFractionDigits=2} .match $n one {{one}} * {{other}}",
        &[],
        "other",
    );
}

/// Alias chains preserve the resolved numeric call and its inherited options.
#[test]
fn numeric_local_alias_chain_preserves_resolved_value() {
    assert_format(
        ".local $n = {4.2 :number minimumFractionDigits=2} .local $alias = {$n} .local $m = {$alias :number signDisplay=always} {{{$m}}}",
        &[],
        "+4.20",
    );
}

/// A dynamic select option stays runtime-resolved: the invalid inner number
/// annotation reports bad-option and the enclosing match reports bad-selector.
#[test]
fn numeric_local_dynamic_select_reports_both_diagnostics() {
    let output = format_output(
        ".local $bad = {exact} .local $n = {1 :number select=$bad} .match $n 1 {{one}} * {{other {$n}}}",
        &[],
    );
    assert_eq!(output.value, "other 1");
    assert_eq!(
        output
            .errors
            .iter()
            .filter(|error| is_bad_option(error))
            .count(),
        1,
        "expected one bad-option diagnostic: {:?}",
        output.errors
    );
    assert_errors_multiset(
        &output.errors,
        &[
            function_error(MessageFunctionError::BadOption),
            FormatError::BadSelector { source: None },
        ],
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — :number selection via select= option
// ---------------------------------------------------------------------------

/// TR35 §14 — :number select=plural: English `one` category for 1.
#[test]
fn number_selection_plural_one() {
    assert_format(
        ".input { $x :number select=plural }\n.match $x\none {{ONE}}\n* {{OTHER}}",
        &[("x", Value::Int(1))],
        "ONE",
    );
}

/// TR35 §14 — :number select=plural: English `other` category for 5.
#[test]
fn number_selection_plural_other() {
    assert_format(
        ".input { $x :number select=plural }\n.match $x\none {{ONE}}\n* {{OTHER}}",
        &[("x", Value::Int(5))],
        "OTHER",
    );
}

/// TR35 §14 — :number defaults to plural selection when `select` is omitted.
#[test]
fn number_selection_default_plural_one() {
    assert_format(
        ".input { $x :number }\n.match $x\none {{ONE}}\n* {{OTHER}}",
        &[("x", Value::Int(1))],
        "ONE",
    );
}

/// TR35 §14 — :number select=exact: exact literal match.
#[test]
fn number_selection_exact() {
    assert_format(
        ".input { $x :number select=exact }\n.match $x\n1 {{ONE}}\n* {{OTHER}}",
        &[("x", Value::Int(1))],
        "ONE",
    );
}

/// TR35 §14 — numeric exact keys outrank keyword keys during plural selection.
#[test]
fn number_selection_plural_exact_key_beats_keyword() {
    assert_format(
        ".input { $x :number select=plural }\n.match $x\none {{KEYWORD}}\n1 {{EXACT}}\n* {{OTHER}}",
        &[("x", Value::Int(1))],
        "EXACT",
    );
}

/// TR35 §14 — :number select=ordinal: English `two` category for 2.
#[test]
fn number_selection_ordinal() {
    assert_format(
        ".input { $x :number select=ordinal }\n.match $x\ntwo {{2ND}}\n* {{OTHER}}",
        &[("x", Value::Int(2))],
        "2ND",
    );
}

/// Dynamic selector modes remain runtime options and therefore produce a bad
/// selector instead of being rewritten to a synthesized exact selector.
#[test]
fn number_selection_dynamic_mode_is_not_synthesized() {
    let output = format_output(
        ".input { $mode }\n.input { $x :number select=$mode }\n.match $x\n1 {{EXACT}}\none {{ONE}}\n* {{OTHER}}",
        &[("mode", Value::Str("plural".into())), ("x", Value::Int(1))],
    );
    assert_eq!(output.value, "OTHER");
    assert_errors_multiset(
        &output.errors,
        &[
            function_error(MessageFunctionError::BadOption),
            FormatError::BadSelector { source: None },
        ],
    );
}

/// TR35 §14 — multi-selector numeric lowering keeps exact-vs-keyword precedence per selector.
#[test]
fn number_selection_two_numeric_selectors() {
    assert_format(
        ".input { $x :number select=plural }\n.input { $y :number select=ordinal }\n.match $x $y\n1 two {{EXACT-TWO}}\none * {{ONE-ANY}}\n* two {{ANY-TWO}}\n* * {{MISS}}",
        &[("x", Value::Int(1)), ("y", Value::Int(2))],
        "EXACT-TWO",
    );
    assert_format(
        ".input { $x :number select=plural }\n.input { $y :number select=ordinal }\n.match $x $y\n1 two {{EXACT-TWO}}\none * {{ONE-ANY}}\n* two {{ANY-TWO}}\n* * {{MISS}}",
        &[("x", Value::Int(2)), ("y", Value::Int(2))],
        "ANY-TWO",
    );
}

/// TR35 §14 — :number select=plural: English maps 0 to `other`.
#[test]
fn plural_selection_zero_english() {
    assert_format_locale(
        ".input { $x :number select=plural }\n.match $x\none {{ONE}}\nother {{OTHER}}\n* {{STAR}}",
        &[("x", Value::Int(0))],
        "en",
        "OTHER",
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — :number signDisplay option (F-15)
// ---------------------------------------------------------------------------

/// F-15 — :number signDisplay=always shows `+` on positive numbers.
#[test]
fn number_sign_display_always() {
    assert_format(
        "{ $x :number signDisplay=always }",
        &[("x", Value::Int(5))],
        "+5",
    );
}

/// F-15 — supported signDisplay values can suppress signs entirely.
#[test]
fn number_sign_display_never() {
    assert_format(
        "{ $x :number signDisplay=never }",
        &[("x", Value::Int(-5))],
        "5",
    );
}

/// F-15 — currently-supported signDisplay values reject unknown literals.
#[test]
fn number_sign_display_invalid_value_is_bad_option() {
    assert_format_err(
        "{ $x :number signDisplay=$mode }",
        &[("x", Value::Int(5)), ("mode", Value::Str("bogus".into()))],
        is_bad_option,
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — :percent function (F-20 via :percent)
// ---------------------------------------------------------------------------

/// F-20 — :percent multiplies by 100 and appends `%`.
#[test]
fn percent_multiplies_by_100() {
    assert_format("{ $x :percent }", &[("x", Value::Float(0.42))], "42%");
}

/// F-20 — :number style=percent should multiply by 100 (spec-canonical form).
#[test]
fn number_style_percent() {
    assert_format(
        "{ $x :number style=percent }",
        &[("x", Value::Float(0.42))],
        "42%",
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — Missing :number options (F-12 through F-19)
// ---------------------------------------------------------------------------

/// F-13 — :number MUST support notation option.
#[test]
fn number_notation_scientific() {
    assert_format(
        "{ $x :number notation=scientific }",
        &[("x", Value::Int(1500))],
        "1.5E3",
    );
}

/// F-17 — :number MUST support useGrouping option.
#[test]
fn number_use_grouping() {
    assert_format(
        "{ $x :number useGrouping=always }",
        &[("x", Value::Int(1000000))],
        "1,000,000",
    );
}

/// F-17 — `min2` only groups when the leading group has at least two digits.
#[test]
fn number_use_grouping_min2() {
    assert_format(
        "{ $x :number useGrouping=min2 }",
        &[("x", Value::Int(1000))],
        "1000",
    );
    assert_format(
        "{ $x :number useGrouping=min2 }",
        &[("x", Value::Int(10000))],
        "10,000",
    );
}

/// F-17 — currently-supported useGrouping values reject unknown literals.
#[test]
fn number_use_grouping_invalid_value_is_bad_option() {
    assert_format_err(
        "{ $x :number useGrouping=$mode }",
        &[
            ("x", Value::Int(1000000)),
            ("mode", Value::Str("bogus".into())),
        ],
        is_bad_option,
    );
}

/// F-18 — :number MUST support minimumIntegerDigits option.
#[test]
fn number_minimum_integer_digits() {
    assert_format(
        "{ $x :number minimumIntegerDigits=3 }",
        &[("x", Value::Int(5))],
        "005",
    );
}

/// F-34 — String value matching number-literal is accepted as :number operand.
#[test]
fn number_string_numeric_literal_accepted() {
    assert_format(
        "{ $x :number }",
        &[("x", Value::Str("42.5".into()))],
        "42.5",
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — :number digit size option errors (F-35, F-37)
// ---------------------------------------------------------------------------

/// F-37 — Negative digit size option value is bad-option.
#[test]
fn digit_size_negative_is_bad_option() {
    assert_format_err(
        "{ $x :number minimumFractionDigits=-1 }",
        &[("x", Value::Int(5))],
        is_bad_option,
    );
}

// ---------------------------------------------------------------------------
// TR35 §14 — :number errors
// ---------------------------------------------------------------------------

/// TR35 §14 — :number with invalid option value is bad-option.
#[test]
fn number_bad_option() {
    assert_format_err(
        "{ $x :number minimumFractionDigits=abc }",
        &[("x", Value::Int(1))],
        is_bad_option,
    );
}

#[test]
fn missing_direct_option_is_recoverable() {
    let output = format_output(
        "{$n :number minimumFractionDigits=$missing}",
        &[("n", Value::Int(1))],
    );
    assert_eq!(output.value, "1");
    assert_errors_multiset(
        &output.errors,
        &[
            missing_arg("missing"),
            function_error(MessageFunctionError::BadOption),
        ],
    );
}

#[test]
fn missing_local_option_is_recoverable() {
    let output = format_output(
        ".local $bad = {$missing} {{{$n :number minimumFractionDigits=$bad}}}",
        &[("n", Value::Int(1))],
    );
    assert_eq!(output.value, "1");
    assert_errors_multiset(
        &output.errors,
        &[
            missing_arg("missing"),
            function_error(MessageFunctionError::BadOption),
        ],
    );
}

#[test]
fn numeric_local_selector_does_not_reresolve_options() {
    let output = format_output(
        ".input {$digits :string} .input {$n :number minimumFractionDigits=$digits} .match $n one {{one}} * {{other}}",
        &[("n", Value::Int(1))],
    );
    assert_eq!(output.value, "one");
    assert_errors_multiset(
        &output.errors,
        &[
            missing_arg("digits"),
            function_error(MessageFunctionError::BadOption),
        ],
    );
}

#[test]
fn missing_dynamic_select_is_unselectable_but_formats() {
    let output = format_output(
        ".local $n = {1 :number select=$missing} .match $n 1 {{wrong}} one {{wrong}} * {{other {$n}}}",
        &[],
    );
    assert_eq!(output.value, "other 1");
    assert_errors_multiset(
        &output.errors,
        &[
            missing_arg("missing"),
            function_error(MessageFunctionError::BadOption),
            FormatError::BadSelector { source: None },
        ],
    );
}

/// TR35 §14 — :number with non-numeric string operand is bad-operand.
#[test]
fn number_bad_string_operand() {
    assert_format_err(
        "{ $x :number }",
        &[("x", Value::Str("not-a-number".into()))],
        is_bad_operand,
    );
}
