// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared pure string-processing utilities used by both compiler and runtime.

use alloc::{
    format,
    string::{String, ToString},
};

pub(crate) fn is_valid_number_literal(value: &str) -> bool {
    let bytes = value.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return false;
    }

    let mut idx = 0_usize;
    if bytes[idx] == b'-' {
        idx += 1;
    }
    if idx >= len {
        return false;
    }

    if bytes[idx] == b'0' {
        idx += 1;
        if idx < len && bytes[idx].is_ascii_digit() {
            return false;
        }
    } else if bytes[idx].is_ascii_digit() {
        idx += 1;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
    } else {
        return false;
    }

    if idx < len && bytes[idx] == b'.' {
        idx += 1;
        let frac_start = idx;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if frac_start == idx {
            return false;
        }
    }

    if idx < len && (bytes[idx] == b'e' || bytes[idx] == b'E') {
        idx += 1;
        if idx < len && (bytes[idx] == b'+' || bytes[idx] == b'-') {
            idx += 1;
        }
        let exp_start = idx;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if exp_start == idx {
            return false;
        }
    }

    idx == len
}

pub(crate) fn parse_number_literal(value: &str) -> Option<f64> {
    if is_valid_number_literal(value) {
        return value.parse::<f64>().ok();
    }
    None
}

pub(crate) fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{2069}'
    )
}

pub(crate) fn strip_bidi_controls(value: &str) -> String {
    value.chars().filter(|ch| !is_bidi_control(*ch)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignDisplay {
    Auto,
    Always,
    Never,
}

pub(crate) fn format_signed_string(sign_display: SignDisplay, value: String) -> String {
    match sign_display {
        SignDisplay::Auto => value,
        SignDisplay::Always => {
            if value.starts_with('-') || value.starts_with('+') {
                value
            } else {
                format!("+{value}")
            }
        }
        SignDisplay::Never => {
            if let Some(stripped) = value.strip_prefix('-').or_else(|| value.strip_prefix('+')) {
                stripped.to_string()
            } else {
                value
            }
        }
    }
}

pub(crate) fn format_signed_number(sign_display: SignDisplay, value: f64) -> String {
    format_signed_string(sign_display, value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_number_literals() {
        assert!(is_valid_number_literal("0"));
        assert!(is_valid_number_literal("42"));
        assert!(is_valid_number_literal("-1"));
        assert!(is_valid_number_literal("3.14"));
        assert!(is_valid_number_literal("-0.5"));
        assert!(is_valid_number_literal("1e10"));
        assert!(is_valid_number_literal("1E10"));
        assert!(is_valid_number_literal("1e+10"));
        assert!(is_valid_number_literal("1e-10"));
        assert!(is_valid_number_literal("1.5e3"));
    }

    #[test]
    fn invalid_number_literals() {
        assert!(!is_valid_number_literal(""));
        assert!(!is_valid_number_literal("-"));
        assert!(!is_valid_number_literal("01"));
        assert!(!is_valid_number_literal(".5"));
        assert!(!is_valid_number_literal("1."));
        assert!(!is_valid_number_literal("1e"));
        assert!(!is_valid_number_literal("+1"));
        assert!(!is_valid_number_literal("abc"));
    }

    #[test]
    fn parse_number_literal_valid() {
        assert_eq!(parse_number_literal("42"), Some(42.0));
        assert_eq!(parse_number_literal("2.75"), Some(2.75));
        assert_eq!(parse_number_literal("-1"), Some(-1.0));
        assert_eq!(parse_number_literal("1e10"), Some(1e10));
    }

    #[test]
    fn parse_number_literal_invalid() {
        assert_eq!(parse_number_literal("abc"), None);
        assert_eq!(parse_number_literal(""), None);
        assert_eq!(parse_number_literal("+1"), None);
    }

    #[test]
    fn bidi_control_detection() {
        assert!(is_bidi_control('\u{061C}'));
        assert!(is_bidi_control('\u{200E}'));
        assert!(is_bidi_control('\u{2069}'));
        assert!(!is_bidi_control('a'));
        assert!(!is_bidi_control(' '));
    }

    #[test]
    fn strip_bidi_controls_removes_all() {
        assert_eq!(strip_bidi_controls("\u{200E}hello\u{200F}"), "hello");
        assert_eq!(strip_bidi_controls("plain"), "plain");
    }

    #[test]
    fn sign_display_auto() {
        assert_eq!(format_signed_string(SignDisplay::Auto, "42".into()), "42");
        assert_eq!(format_signed_string(SignDisplay::Auto, "-5".into()), "-5");
    }

    #[test]
    fn sign_display_always() {
        assert_eq!(
            format_signed_string(SignDisplay::Always, "42".into()),
            "+42"
        );
        assert_eq!(format_signed_string(SignDisplay::Always, "-5".into()), "-5");
        assert_eq!(format_signed_string(SignDisplay::Always, "+3".into()), "+3");
    }

    #[test]
    fn sign_display_never() {
        assert_eq!(format_signed_string(SignDisplay::Never, "-5".into()), "5");
        assert_eq!(format_signed_string(SignDisplay::Never, "+3".into()), "3");
        assert_eq!(format_signed_string(SignDisplay::Never, "42".into()), "42");
    }

    #[test]
    fn format_signed_number_uses_shortest_repr() {
        assert_eq!(
            format_signed_number(SignDisplay::Auto, 1e23),
            "100000000000000000000000"
        );
        assert_eq!(format_signed_number(SignDisplay::Auto, 2.75), "2.75");
        assert_eq!(format_signed_number(SignDisplay::Always, 0.0), "+0");
        assert_eq!(format_signed_number(SignDisplay::Never, -0.0), "0");
    }
}
