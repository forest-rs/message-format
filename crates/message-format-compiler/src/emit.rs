// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! MF2 source emission helpers.
//!
//! Helpers that turn plain Rust strings into valid `MessageFormat` 2 source
//! text. These are the inverse of the syntax-decoding helpers in
//! [`crate::syntax`]: rather than parse an MF2 pattern into decoded text,
//! they take arbitrary text and produce MF2 source that round-trips back
//! to that same text.

use std::borrow::Cow;

use crate::syntax::ident::is_ignorable_char;

/// Escape plain text for embedding in an MF2 pattern.
///
/// Takes an arbitrary plain string and returns valid MF2 `text` source by
/// escaping `\` as `\\`, `{` as `\{`, and `}` as `\}`. The pipe character `|`
/// is a valid `text-char` and is not escaped.
///
/// If the first non-whitespace/non-bidi character is `.`, an empty literal
/// `{||}` is prepended to disambiguate the result from a complex-message
/// declaration prelude. Whitespace and bidi controls are MF2 `s`
/// (`%x09 %x0A %x0D %x20 %x3000`) and bidi marks
/// (`%x061C %x200E %x200F %x2066–2069`), matching the parser's
/// `skip_optional_whitespace`.
///
/// Returns [`Cow::Borrowed`] when no escaping is needed, avoiding allocation on
/// the common path.
pub fn escape_text(value: &str) -> Cow<'_, str> {
    let needs_prefix = value.chars().find(|&ch| !is_ignorable_char(ch)) == Some('.');
    let interior_extra: usize = value
        .chars()
        .filter(|ch| matches!(ch, '\\' | '{' | '}'))
        .count();
    let extra = if needs_prefix { 4 } else { 0 } + interior_extra;
    if extra == 0 {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + extra);
    if needs_prefix {
        out.push_str("{||}");
    }
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str(r"\\"),
            '{' => out.push_str(r"\{"),
            '}' => out.push_str(r"\}"),
            _ => out.push(ch),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::format_simple_main;

    #[test]
    fn escape_text_plain_passthrough() {
        assert_eq!(escape_text("hello world"), "hello world");
    }

    #[test]
    fn escape_text_backslash() {
        assert_eq!(escape_text(r"a\b"), r"a\\b");
    }

    #[test]
    fn escape_text_open_brace() {
        assert_eq!(escape_text("a{b"), r"a\{b");
    }

    #[test]
    fn escape_text_close_brace() {
        assert_eq!(escape_text("a}b"), r"a\}b");
    }

    #[test]
    fn escape_text_combined() {
        assert_eq!(escape_text(r"a\{b}c"), r"a\\\{b\}c");
    }

    #[test]
    fn escape_text_pipe_not_escaped() {
        assert_eq!(escape_text("a|b"), "a|b");
    }

    #[test]
    fn escape_text_empty() {
        assert_eq!(escape_text(""), "");
    }

    #[test]
    fn escape_text_unicode_passthrough() {
        assert_eq!(escape_text("héllo 世界 🎉"), "héllo 世界 🎉");
    }

    #[test]
    fn escape_text_leading_dot_simple() {
        assert_eq!(escape_text(".hello"), "{||}.hello");
    }

    #[test]
    fn escape_text_leading_dot_alone() {
        assert_eq!(escape_text("."), "{||}.");
    }

    #[test]
    fn escape_text_interior_dot_untouched() {
        assert_eq!(escape_text("a.b"), "a.b");
    }

    #[test]
    fn escape_text_leading_dot_with_brace() {
        assert_eq!(escape_text(".a{b"), r"{||}.a\{b");
    }

    #[test]
    fn escape_text_leading_dot_then_newline() {
        assert_eq!(escape_text(".\n"), "{||}.\n");
    }

    #[test]
    fn escape_text_leading_space_dot() {
        assert_eq!(escape_text(" .foo"), "{||} .foo");
    }

    #[test]
    fn escape_text_leading_tab_dot() {
        assert_eq!(escape_text("\t.foo"), "{||}\t.foo");
    }

    #[test]
    fn escape_text_leading_ideographic_space_dot() {
        assert_eq!(escape_text("\u{3000}.baz"), "{||}\u{3000}.baz");
    }

    #[test]
    fn escape_text_leading_bidi_mark_dot() {
        assert_eq!(escape_text("\u{200E}.bar"), "{||}\u{200E}.bar");
    }

    #[test]
    fn escape_text_mixed_ws_bidi_before_dot() {
        assert_eq!(escape_text(" \u{200E}.foo"), "{||} \u{200E}.foo");
    }

    #[test]
    fn escape_text_leading_whitespace_no_dot_passthrough() {
        assert_eq!(escape_text(" hello"), " hello");
    }

    #[test]
    fn escape_text_all_whitespace_passthrough() {
        assert_eq!(escape_text("   "), "   ");
    }

    #[test]
    fn escape_text_bidi_only_passthrough() {
        assert_eq!(escape_text("\u{200E}\u{200F}"), "\u{200E}\u{200F}");
    }

    #[test]
    fn escape_text_pipeline_round_trip() {
        let inputs = &[
            "hello",
            r"back\slash",
            "open{brace",
            "close}brace",
            "a|pipe",
            ".dotted",
            ".",
            ".{brace",
            "",
            " .foo",
            "\t.foo",
            "\u{3000}.baz",
            "\u{200E}.bar",
            " hello",
        ];
        for &input in inputs {
            let escaped = escape_text(input);
            let formatted = format_simple_main(&escaped);
            assert_eq!(formatted, input, "round-trip failed for {input:?}");
        }
    }
}
