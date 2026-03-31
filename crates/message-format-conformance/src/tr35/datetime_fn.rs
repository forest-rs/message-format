// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::Value;

// ---------------------------------------------------------------------------
// TR35 §15 — :date function
// ---------------------------------------------------------------------------

/// TR35 §15 — :date accepts a date literal and formats it.
#[test]
fn date_accepts_literal() {
    let out = format("{|2024-05-01| :date}", &[]);
    assert!(
        out.contains("2024") && out.contains("May"),
        "expected formatted date, got: {out}",
    );
}

/// TR35 §15 — :date defaults to style=medium locale-aware output.
#[test]
fn date_default_medium_style() {
    let out = format_locale("{|2024-05-01| :date}", &[], "en");
    assert!(
        out.contains("May") && out.contains("2024"),
        "expected medium date format like 'May 1, 2024', got: {out}",
    );
}

/// TR35 §15 — :date with explicit style=long.
#[test]
fn date_long_style() {
    let out = format_locale("{|2024-05-01| :date style=long}", &[], "en");
    assert!(
        out.contains("May") && out.contains("2024"),
        "expected long date format, got: {out}",
    );
}

/// TR35 §15 — :date with explicit style=short.
#[test]
fn date_short_style() {
    let out = format_locale("{|2024-05-01| :date style=short}", &[], "en");
    // Short date: "5/1/24" or similar
    assert!(
        out.contains("5") && out.contains("24"),
        "expected short date format, got: {out}",
    );
}

/// TR35 §15 — :date with a full ISO 8601 datetime literal extracts the date.
#[test]
fn date_from_datetime_literal() {
    let out = format("{|2024-05-01T14:30:00| :date}", &[]);
    assert!(
        out.contains("2024"),
        "expected date from datetime, got: {out}",
    );
}

// ---------------------------------------------------------------------------
// TR35 §15 — :time function
// ---------------------------------------------------------------------------

/// TR35 §15 — :time formats a time from a datetime literal.
#[test]
fn time_from_datetime_literal() {
    let out = format("{|2024-05-01T14:30:00| :time}", &[]);
    assert!(
        !out.is_empty(),
        "expected non-empty time output, got: {out}"
    );
}

/// TR35 §15 — :time defaults to style=short locale-aware output.
#[test]
fn time_default_short_style() {
    let out = format_locale("{|2024-05-01T14:30:00| :time}", &[], "en");
    // Short style: "2:30 PM" or "14:30"
    assert!(
        out.contains("2:30") || out.contains("14:30"),
        "expected short time format, got: {out}",
    );
}

/// TR35 §15 — :time with a date-only literal defaults to 00:00:00.
#[test]
fn time_from_date_only_defaults_to_midnight() {
    let out = format("{|2024-05-01| :time}", &[]);
    assert!(
        out.contains("00") || out.contains("12:00"),
        "expected midnight time, got: {out}",
    );
}

// ---------------------------------------------------------------------------
// TR35 §15 — :datetime function
// ---------------------------------------------------------------------------

/// TR35 §15 — :datetime formats a full datetime literal.
#[test]
fn datetime_full_literal() {
    let out = format("{|2024-05-01T14:30:00| :datetime}", &[]);
    assert!(
        !out.is_empty(),
        "expected non-empty datetime output, got: {out}"
    );
}

/// TR35 §15 — :datetime defaults to dateStyle=medium timeStyle=short.
#[test]
fn datetime_default_format() {
    let out = format_locale("{|2024-05-01T14:30:00| :datetime}", &[], "en");
    assert!(
        out.contains("May") && out.contains("30"),
        "expected medium date + short time, got: {out}",
    );
}

/// F-52/F-53 — :datetime style and field options are mutually exclusive.
#[test]
fn datetime_style_and_field_mutually_exclusive() {
    assert_format_err(
        "{ |2024-05-01T14:30:00| :datetime dateStyle=medium year=numeric }",
        &[],
        is_bad_option,
    );
}

/// F-63 — :date/:time/:datetime accept ISO 8601 datetime with timezone offset.
#[test]
fn datetime_iso8601_with_offset() {
    let out = format("{|2024-05-01T14:30:00+05:30| :datetime}", &[]);
    assert!(
        !out.is_empty(),
        "expected non-empty datetime output, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// TR35 §15 — :date, :time, :datetime errors
// ---------------------------------------------------------------------------

/// TR35 §15 — :date with a non-date operand is bad-operand.
#[test]
fn date_bad_operand() {
    assert_format_err("{ $x :date }", &[("x", Value::Bool(true))], is_bad_operand);
}

/// TR35 §15 — :datetime with a non-date operand is bad-operand.
#[test]
fn datetime_bad_operand() {
    assert_format_err(
        "{ $x :datetime }",
        &[("x", Value::Bool(true))],
        is_bad_operand,
    );
}

/// TR35 §15 — :time with a non-date operand is bad-operand.
#[test]
fn time_bad_operand() {
    assert_format_err("{ $x :time }", &[("x", Value::Bool(true))], is_bad_operand);
}

/// TR35 §15 — :date with a malformed date string is bad-operand.
#[test]
fn date_bad_string_operand() {
    assert_format_err(
        "{ $x :date }",
        &[("x", Value::Str("not-a-date".into()))],
        is_bad_operand,
    );
}

/// TR35 §15 — :date with an integer operand is bad-operand.
#[test]
fn date_integer_operand() {
    assert_format_err("{ $x :date }", &[("x", Value::Int(42))], is_bad_operand);
}
