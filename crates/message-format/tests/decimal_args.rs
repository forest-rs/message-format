// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration coverage for exact fixed-decimal arguments.

#![cfg(feature = "compile")]

use core::str::FromStr;

use fixed_decimal::Decimal;
use message_format::runtime::{BuiltinHost, Formatter, MessageArgs as RuntimeMessageArgs};
use message_format::{Catalog, Locale, MessageArgs};

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("valid fixed decimal")
}

fn facade_format(
    source: &str,
    value: impl Into<message_format::runtime::Value>,
    locale: &str,
) -> String {
    let catalog = Catalog::compile_str(source).expect("compile");
    let locale = locale.parse::<Locale>().expect("locale");
    let mut formatter = catalog
        .formatter_for_locale(&locale)
        .expect("facade formatter");
    let mut args = MessageArgs::new();
    args.insert("amount", value);
    formatter
        .format_by_id("main", &args)
        .expect("facade format")
}

fn runtime_format(
    source: &str,
    value: impl Into<message_format::runtime::Value>,
    locale: &str,
) -> String {
    let catalog = Catalog::compile_str(source).expect("compile");
    let locale = locale.parse::<Locale>().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("runtime formatter");
    let message = formatter.resolve("main").expect("message");
    let mut args = RuntimeMessageArgs::new(&catalog);
    args.insert("amount", value).expect("known argument");
    let mut output = String::new();
    formatter
        .format_to(message, &args, &mut output, None)
        .expect("runtime format");
    output
}

#[test]
fn facade_args_accept_owned_and_borrowed_decimals_exactly() {
    let borrowed = decimal("1.00");
    assert_eq!(facade_format("{ $amount }", &borrowed, "en"), "1.00");

    assert_eq!(
        facade_format("{ $amount }", decimal("-0.00"), "en"),
        "-0.00"
    );
    assert_eq!(
        facade_format("{ $amount }", decimal("-12345678901234567890.0100"), "en",),
        "-12,345,678,901,234,567,890.0100"
    );
}

#[test]
fn runtime_args_accept_owned_and_borrowed_decimals_exactly() {
    let borrowed = decimal("-0.00");
    assert_eq!(runtime_format("{ $amount }", &borrowed, "en"), "-0.00");
    assert_eq!(
        runtime_format("{ $amount }", decimal("1.2300"), "en"),
        "1.2300"
    );
}

#[test]
fn decimal_parameters_use_locale_formatting_without_losing_precision() {
    let amount = decimal("12345678901234567.8900");
    assert_eq!(
        facade_format("{ $amount :number }", &amount, "en"),
        "12,345,678,901,234,567.8900"
    );
    assert_eq!(
        facade_format("{ $amount :number }", &amount, "fr"),
        "12\u{202f}345\u{202f}678\u{202f}901\u{202f}234\u{202f}567,8900"
    );
}

#[test]
fn decimal_parameters_honor_number_precision_and_numbering_options() {
    assert_eq!(
        runtime_format(
            "{ $amount :number maximumFractionDigits=2 }",
            decimal("12.3450"),
            "en",
        ),
        "12.35"
    );
    assert_eq!(
        runtime_format(
            "{ $amount :number minimumSignificantDigits=4 }",
            decimal("12.3"),
            "en",
        ),
        "12.30"
    );
    assert_eq!(
        runtime_format(
            "{ $amount :number numberingSystem=arab }",
            decimal("12345.60"),
            "en",
        ),
        "١٢,٣٤٥.٦٠"
    );
}

#[test]
fn decimal_parameters_work_with_percent_currency_and_offset() {
    assert_eq!(
        runtime_format(
            "{ $amount :percent maximumFractionDigits=19 }",
            decimal("0.1234567890123456789"),
            "en",
        ),
        "12.34567890123456789%"
    );
    assert_eq!(
        runtime_format(
            "{ $amount :currency currency=USD }",
            decimal("9007199254740993.25"),
            "en-US",
        ),
        "$9,007,199,254,740,993.25"
    );
    assert_eq!(
        runtime_format(
            "{ $amount :offset subtract=1 }",
            decimal("9007199254740993.25"),
            "en",
        ),
        "9,007,199,254,740,992.25"
    );
}

#[test]
fn decimal_scale_participates_in_plural_selection() {
    let source = ".input { $amount :number select=plural }\n\
                  .match $amount\n\
                  one {{one}}\n\
                  * {{other}}";

    assert_eq!(facade_format(source, decimal("1"), "en"), "one");
    assert_eq!(facade_format(source, decimal("1.00"), "en"), "other");
}

#[test]
fn decimal_offset_defaults_to_plural_selection() {
    let source = ".input { $amount :offset subtract=1 }\n\
                  .match $amount\n\
                  one {{one}}\n\
                  * {{other}}";

    assert_eq!(facade_format(source, decimal("2.0"), "en"), "one");
    assert_eq!(runtime_format(source, decimal("2.0"), "en"), "one");
}

#[test]
fn decimal_exact_selection_uses_one_canonical_spelling() {
    let equivalent_keys = ".input { $amount :number select=exact }\n\
                           .match $amount\n\
                           1.0 {{fractional spelling}}\n\
                           1e0 {{exponent spelling}}\n\
                           1 {{canonical spelling}}\n\
                           * {{other}}";
    for amount in [decimal("1"), decimal("1.00"), decimal("1e0")] {
        assert_eq!(
            facade_format(equivalent_keys, amount, "en"),
            "canonical spelling"
        );
    }

    let fraction = ".input { $amount :number select=exact }\n\
                    .match $amount\n\
                    1.2300 {{retained scale}}\n\
                    123e-2 {{exponent spelling}}\n\
                    1.23 {{canonical spelling}}\n\
                    * {{other}}";
    assert_eq!(
        facade_format(fraction, decimal("1.2300"), "en"),
        "canonical spelling"
    );

    let zero = ".input { $amount :number select=exact }\n\
                .match $amount\n\
                -0 {{negative zero}}\n\
                0.0 {{fractional zero}}\n\
                0 {{canonical zero}}\n\
                * {{other}}";
    assert_eq!(
        facade_format(zero, decimal("-0.00"), "en"),
        "canonical zero"
    );

    let plural = ".input { $amount :number select=plural }\n\
                  .match $amount\n\
                  1 {{exact}}\n\
                  one {{one}}\n\
                  * {{other}}";
    assert_eq!(facade_format(plural, decimal("1.00"), "en"), "exact");
}

#[test]
fn numeric_transformations_match_the_canonical_transformed_payload() {
    for (source, amount, expected) in [
        (
            ".input {$amount :integer select=exact} .match $amount 1.0 {{noncanonical}} 1 {{integer}} * {{other}}",
            decimal("1.99"),
            "integer",
        ),
        (
            ".input {$amount :offset subtract=1 select=exact} .match $amount 1.0 {{noncanonical}} 1 {{offset}} * {{other}}",
            decimal("2.00"),
            "offset",
        ),
        (
            ".input {$amount :percent select=exact} .match $amount 1.0 {{noncanonical}} 1 {{percent}} * {{other}}",
            decimal("0.0100"),
            "percent",
        ),
    ] {
        assert_eq!(facade_format(source, amount, "en"), expected);
    }
}

#[test]
fn display_options_do_not_change_numeric_exact_serialization() {
    for (source, amount) in [
        (
            ".input {$amount :number select=exact minimumFractionDigits=4 minimumIntegerDigits=6 useGrouping=always notation=scientific numberingSystem=arab signDisplay=always}\n\
             .match $amount 1.2345 {{payload}} 1.23450 {{display precision}} * {{other}}",
            decimal("1.2345"),
        ),
        (
            ".input {$amount :number select=exact maximumFractionDigits=2}\n\
             .match $amount 1.23456 {{payload}} 1.23 {{display precision}} * {{other}}",
            decimal("1.23456"),
        ),
        (
            ".input {$amount :number select=exact maximumSignificantDigits=3}\n\
             .match $amount 12345 {{payload}} 12300 {{display precision}} * {{other}}",
            decimal("12345"),
        ),
    ] {
        assert_eq!(facade_format(source, amount, "en"), "payload");
    }
}

#[test]
fn string_selection_of_decimal_parameters_remains_textual() {
    let source = ".input { $amount :string }\n\
                  .match $amount\n\
                  1 {{integer text}}\n\
                  1.00 {{decimal text}}\n\
                  * {{other}}";

    assert_eq!(facade_format(source, decimal("1.00"), "en"), "decimal text");
}

#[test]
fn callers_can_explicitly_trim_decimal_scale() {
    let mut amount = decimal("1.00");
    amount.trim_end();

    assert_eq!(facade_format("{ $amount }", amount, "en"), "1");
}
