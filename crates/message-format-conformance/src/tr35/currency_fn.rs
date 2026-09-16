// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;

#[test]
fn currency_uses_locale_symbol_and_currency_precision() {
    assert_eq!(
        format_locale("{42 :currency currency=EUR}", &[], "en-US"),
        "€42.00"
    );
    assert_eq!(
        format_locale("{42 :currency currency=EUR}", &[], "fr-FR"),
        "42,00\u{a0}€"
    );
}

#[test]
fn currency_formats_negative_and_accounting_values() {
    assert_eq!(
        format_locale("{-42 :currency currency=USD}", &[], "en-US"),
        "-$42.00"
    );
    assert_eq!(
        format_locale(
            "{-42 :currency currency=USD currencySign=accounting}",
            &[],
            "en-US",
        ),
        "($42.00)"
    );
}

#[test]
fn currency_display_options_select_icu_formatters() {
    let code = format_locale(
        "{42 :currency currency=EUR currencyDisplay=code}",
        &[],
        "en-US",
    );
    assert!(code.starts_with("EUR") && code.ends_with("42.00"), "{code}");

    let name = format_locale(
        "{42 :currency currency=EUR currencyDisplay=name}",
        &[],
        "en-US",
    );
    assert_eq!(name, "42.00 euros");

    assert_eq!(
        format_locale(
            "{42 :currency currency=EUR currencyDisplay=never}",
            &[],
            "en-US",
        ),
        "42.00"
    );
}

#[test]
fn currency_reannotation_inherits_resolved_options() {
    let out = format_locale(
        ".local $n = {42 :currency currency=EUR currencyDisplay=code} {{{$n :currency}}}",
        &[],
        "en-US",
    );
    assert!(out.starts_with("EUR") && out.ends_with("42.00"), "{out}");
}

#[test]
fn currency_requires_a_valid_currency_code() {
    assert_format_err("{42 :currency}", &[], is_bad_operand);
    assert_format_err("{42 :currency currency=EURO}", &[], is_bad_option);
}
