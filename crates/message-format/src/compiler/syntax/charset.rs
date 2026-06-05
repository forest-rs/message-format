// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! ABNF-precise character predicates for the MF2 grammar.
//!
//! Each predicate corresponds to a named production in the TR35 ABNF.

/// ABNF `name-start`: `[a-zA-Z_]` plus Unicode letter ranges from the spec.
///
/// ```text
/// name-start = ALPHA / "_"
///            / %xC0-D6 / %xD8-F6 / %xF8-2FF
///            / %x370-37D / %x37F-61B / %x61D-1FFF
///            / %x200C-200D / %x2070-218F / %x2C00-2FEF
///            / %x3001-D7FF / %xF900-FDCF / %xFDF0-FFFC
///            / %x10000-EFFFF
/// ```
pub(crate) fn is_name_start(ch: char) -> bool {
    let cp = ch as u32;
    matches!(ch, 'a'..='z' | 'A'..='Z' | '_')
        || matches!(cp,
            0xC0..=0xD6
            | 0xD8..=0xF6
            | 0xF8..=0x2FF
            | 0x370..=0x37D
            | 0x37F..=0x61B
            | 0x61D..=0x1FFF
            | 0x200C..=0x200D
            | 0x2070..=0x218F
            | 0x2C00..=0x2FEF
            | 0x3001..=0xD7FF
            | 0xF900..=0xFDCF
            | 0xFDF0..=0xFFFC
            | 0x10000..=0xEFFFF
        )
}

/// ABNF `name-char`: `name-start` plus digits, `-`, `.`, middle-dot, combining ranges.
///
/// ```text
/// name-char  = name-start / DIGIT / "-" / "."
///            / %xB7 / %x0300-036F / %x203F-2040
/// ```
pub(crate) fn is_name_char(ch: char) -> bool {
    if is_name_start(ch) {
        return true;
    }
    let cp = ch as u32;
    ch.is_ascii_digit()
        || ch == '-'
        || ch == '.'
        || cp == 0xB7
        || matches!(cp, 0x0300..=0x036F | 0x203F..=0x2040)
}

/// ABNF `content-char`: characters allowed in pattern text outside of
/// whitespace, escapes, and placeholders.
///
/// Excludes: NUL, HTAB, LF, CR, SP, `.`, `@`, `\`, `{`, `|`, `}`,
/// U+3000 (ideographic space), surrogates, and noncharacters.
pub(crate) fn is_content_char(ch: char) -> bool {
    if matches!(ch, '\\' | '{' | '}' | '.' | '@' | '|') {
        return false;
    }
    let cp = ch as u32;
    if cp == 0 {
        return false;
    }
    // NUL, HTAB, LF, CR, SP, ideographic space
    if matches!(cp, 0x09 | 0x0A | 0x0D | 0x20 | 0x3000) {
        return false;
    }
    // Unicode noncharacters
    if is_noncharacter(cp) {
        return false;
    }
    true
}

/// ABNF `text-char = content-char / ws / "." / "@" / "|"`.
///
/// Note: `}` is NOT a text-char per ABNF but the parser handles `}` as
/// literal text in simple messages via a separate path.
#[allow(
    dead_code,
    reason = "Kept for ABNF correspondence and future parser checks."
)]
pub(crate) fn is_text_char(ch: char) -> bool {
    is_content_char(ch) || is_mf2_whitespace(ch) || matches!(ch, '.' | '@' | '|')
}

/// ABNF `quoted-char = content-char / ws / "." / "@" / "{" / "}"`.
///
/// Note: `|` is NOT in quoted-char (it is the delimiter).
pub(crate) fn is_quoted_char(ch: char) -> bool {
    is_content_char(ch) || is_mf2_whitespace(ch) || matches!(ch, '.' | '@' | '{' | '}')
}

/// ABNF `simple-start-char = content-char / "@" / "|"`.
///
/// Note: `.` is NOT a simple-start-char (it starts declarations).
#[allow(
    dead_code,
    reason = "Kept for ABNF correspondence and future parser checks."
)]
pub(crate) fn is_simple_start_char(ch: char) -> bool {
    is_content_char(ch) || matches!(ch, '@' | '|')
}

/// MF2 whitespace: `s = SP / HTAB / CR / LF / %x3000`
pub(crate) fn is_mf2_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\r' | '\n' | '\u{3000}')
}

fn is_noncharacter(cp: u32) -> bool {
    (0xFDD0..=0xFDEF).contains(&cp) || (cp & 0xFFFE) == 0xFFFE
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── is_content_char ──────────────────────────────────────────────

    #[test]
    fn content_char_excludes_sp() {
        assert!(!is_content_char(' '));
    }

    #[test]
    fn content_char_excludes_dot() {
        assert!(!is_content_char('.'));
    }

    #[test]
    fn content_char_excludes_at() {
        assert!(!is_content_char('@'));
    }

    #[test]
    fn content_char_excludes_pipe() {
        assert!(!is_content_char('|'));
    }

    #[test]
    fn content_char_excludes_ideographic_space() {
        assert!(!is_content_char('\u{3000}'));
    }

    #[test]
    fn content_char_excludes_backslash_braces() {
        assert!(!is_content_char('\\'));
        assert!(!is_content_char('{'));
        assert!(!is_content_char('}'));
    }

    #[test]
    fn content_char_excludes_controls() {
        assert!(!is_content_char('\0'));
        assert!(!is_content_char('\t'));
        assert!(!is_content_char('\n'));
        assert!(!is_content_char('\r'));
    }

    #[test]
    fn content_char_allows_normal_chars() {
        assert!(is_content_char('!'));
        assert!(is_content_char('a'));
        assert!(is_content_char('0'));
        assert!(is_content_char('Z'));
        // Non-ASCII letter
        assert!(is_content_char('é'));
        // CJK character
        assert!(is_content_char('漢'));
    }

    // ─── is_text_char ─────────────────────────────────────────────────

    #[test]
    fn text_char_allows_content_char_plus_ws_dot_at_pipe() {
        assert!(is_text_char('a'));
        assert!(is_text_char(' '));
        assert!(is_text_char('\t'));
        assert!(is_text_char('.'));
        assert!(is_text_char('@'));
        assert!(is_text_char('|'));
    }

    #[test]
    fn text_char_excludes_backslash_open_brace() {
        assert!(!is_text_char('\\'));
        assert!(!is_text_char('{'));
    }

    // ─── is_quoted_char ───────────────────────────────────────────────

    #[test]
    fn quoted_char_allows_braces_dot_at() {
        assert!(is_quoted_char('{'));
        assert!(is_quoted_char('}'));
        assert!(is_quoted_char('.'));
        assert!(is_quoted_char('@'));
    }

    #[test]
    fn quoted_char_excludes_pipe() {
        assert!(!is_quoted_char('|'));
    }

    // ─── is_simple_start_char ─────────────────────────────────────────

    #[test]
    fn simple_start_char_allows_at_and_pipe() {
        assert!(is_simple_start_char('@'));
        assert!(is_simple_start_char('|'));
    }

    #[test]
    fn simple_start_char_excludes_dot() {
        assert!(!is_simple_start_char('.'));
    }
}
