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
fn callers_can_explicitly_trim_decimal_scale() {
    let mut amount = decimal("1.00");
    amount.trim_end();

    assert_eq!(facade_format("{ $amount }", amount, "en"), "1");
}
