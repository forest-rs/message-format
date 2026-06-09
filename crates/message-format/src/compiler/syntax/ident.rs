// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared identifier and bidi-control helpers for syntax and semantic passes.

use alloc::string::{String, ToString};

/// Returns `true` when `value` is a valid MF2 identifier.
pub(crate) fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_identifier_start_char(first) {
        return false;
    }
    chars.all(is_identifier_continue_char)
}

/// Returns `true` when `ch` is a valid identifier start character.
///
/// Uses the ABNF `name-start` production for precise Unicode coverage.
pub(crate) fn is_identifier_start_char(ch: char) -> bool {
    crate::compiler::syntax::charset::is_name_start(ch)
}

/// Returns `true` when `ch` is a valid identifier continuation character.
///
/// Uses the ABNF `name-char` production for precise Unicode coverage.
pub(crate) fn is_identifier_continue_char(ch: char) -> bool {
    crate::compiler::syntax::charset::is_name_char(ch)
}

use crate::common::text::{is_bidi_control, strip_bidi_controls};

/// Returns `true` for MF2 whitespace and bidi controls.
pub(crate) fn is_ignorable_char(ch: char) -> bool {
    crate::compiler::syntax::charset::is_mf2_whitespace(ch) || is_bidi_control(ch)
}

/// Canonicalize an identifier by stripping bidi controls and applying NFC normalization.
pub(crate) fn canonicalize_identifier(value: &str) -> String {
    let stripped = strip_bidi_controls(value);
    let normalizer = icu_normalizer::ComposingNormalizer::new_nfc();
    normalizer.normalize(&stripped).to_string()
}

/// Trim boundary whitespace/bidi controls and reject embedded bidi controls.
pub(crate) fn strip_boundary_bidi_controls(value: &str) -> Option<String> {
    let stripped = value.trim_matches(is_ignorable_char);
    if stripped.chars().any(is_bidi_control) {
        return None;
    }
    Some(stripped.to_string())
}
