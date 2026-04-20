// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Literal and text decoding helpers used by parser/semantic/lowering stages.

use crate::compile::CompileError;
use crate::syntax::ident::is_ignorable_char;
use crate::syntax::span::{char_column, quoted_snippet};
use icu_normalizer::ComposingNormalizer;

/// Parse a literal token (quoted or unquoted) into decoded text.
///
/// The result is NFC-normalized per TR35: keys MUST be in NFC, and source
/// literals SHOULD be in NFC.  Normalizing here ensures consistent matching.
pub(crate) fn parse_literal_text(value: &str, line: usize) -> Result<String, CompileError> {
    let decoded = if value.starts_with('|') {
        if !value.ends_with('|') || value.len() < 2 {
            return Err(CompileError::invalid_expr(line));
        }
        decode_quoted_literal(&value[1..value.len() - 1], line)?
    } else {
        if !is_unquoted_literal_token(value) {
            return Err(CompileError::invalid_expr(line));
        }
        value.to_string()
    };
    Ok(normalize_nfc(&decoded))
}

/// Apply Unicode NFC normalization.
fn normalize_nfc(value: &str) -> String {
    let normalizer = ComposingNormalizer::new_nfc();
    normalizer.normalize(value).to_string()
}

/// Decode text fragment escapes used in patterns.
///
/// ABNF `escaped-char = backslash ( backslash / "{" / "|" / "}" )`
pub(crate) fn decode_text_fragment(value: &str, line: usize) -> Result<String, CompileError> {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            if !matches!(ch, '\\' | '{' | '|' | '}') {
                return Err(CompileError::invalid_expr_detail(
                    line,
                    0,
                    "valid escape (\\\\, \\{, \\|, or \\})",
                    quoted_snippet(&format!("\\{ch}")),
                ));
            }
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        out.push(ch);
    }
    if escaped {
        return Err(CompileError::invalid_expr(line));
    }
    Ok(out)
}

/// Validate braces in a raw pattern body before lowering expression spans.
pub(crate) fn validate_balanced_braces(template: &str, line: usize) -> Result<(), CompileError> {
    let mut depth = 0_i32;
    let mut escaped = false;
    for (idx, ch) in template.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return Err(CompileError::invalid_expr_detail(
                        line,
                        char_column(template, idx),
                        "matching '{' before '}'",
                        "'}'",
                    ));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(CompileError::invalid_expr_detail(
            line,
            char_column(template, template.len()),
            "matching '}'",
            "unclosed '{'",
        ))
    }
}

/// Validate that a quoted raw body, if present, is wrapped as `{{...}}`.
pub(crate) fn ensure_well_formed_quoted_pattern_body(
    body: &str,
    line: usize,
) -> Result<(), CompileError> {
    let trimmed = body.trim_matches(is_ignorable_char);
    if !trimmed.contains("{{") {
        return Ok(());
    }
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return Err(CompileError::invalid_expr_detail(
            line,
            1,
            "quoted pattern body wrapped by '{{' and '}}'",
            quoted_snippet(trimmed),
        ));
    }
    Ok(())
}

/// Parse a number literal token using MF2 lexical rules.
pub(crate) fn parse_number_literal(value: &str) -> Option<f64> {
    if is_valid_number_literal(value) {
        return value.parse::<f64>().ok();
    }
    None
}

/// ABNF `escaped-char` in quoted literals: `backslash ( backslash / "{" / "|" / "}" )`
fn decode_quoted_literal(value: &str, line: usize) -> Result<String, CompileError> {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            if !matches!(ch, '\\' | '{' | '|' | '}') {
                return Err(CompileError::invalid_expr_detail(
                    line,
                    0,
                    "valid escape (\\\\, \\{, \\|, or \\})",
                    quoted_snippet(&format!("\\{ch}")),
                ));
            }
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '|' {
            return Err(CompileError::invalid_expr(line));
        } else {
            out.push(ch);
        }
    }
    if escaped {
        return Err(CompileError::invalid_expr(line));
    }
    Ok(out)
}

fn is_unquoted_literal_token(value: &str) -> bool {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return false;
    }
    if value.contains('\\') {
        return false;
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || is_noncharacter(ch))
    {
        return false;
    }
    if is_valid_number_literal(value) {
        return true;
    }
    if value.contains('@') {
        return false;
    }
    if value.contains(':') {
        return false;
    }
    let first = value.chars().next().unwrap_or_default();
    if "^!%*<>?~&".contains(first) {
        return false;
    }
    if first == '+' && value.len() > 1 {
        return false;
    }
    true
}

fn is_noncharacter(ch: char) -> bool {
    let cp = ch as u32;
    (0xFDD0..=0xFDEF).contains(&cp) || (cp & 0xFFFE) == 0xFFFE
}

fn is_valid_number_literal(value: &str) -> bool {
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
