// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared byte-offset → line/column mapping and snippet utilities.

/// Convert a byte index into a 1-based character column within `source`.
///
/// The entire `source` slice is treated as a single line; use [`byte_to_line_col`]
/// when line information is also needed.
pub(crate) fn char_column(source: &str, byte_idx: usize) -> usize {
    let safe = byte_idx.min(source.len());
    source[..safe].chars().count() + 1
}

/// Convert a byte index into 1-based `(line, column)` coordinates.
pub(crate) fn byte_to_line_col(source: &str, byte_idx: usize) -> (usize, usize) {
    let mut line = 1_usize;
    let mut line_start = 0_usize;
    let target = byte_idx.min(source.len());
    for (idx, ch) in source.char_indices() {
        if idx >= target {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + ch.len_utf8();
        }
    }
    let column = source[line_start..target].chars().count() + 1;
    (line, column)
}

/// Diagnostic source-location strategy.
///
/// In multi-message mode the line is known up-front (one message per line),
/// so we only need a character-column offset.  In single-message / raw-document
/// mode the byte offset is converted to `(line, column)` on the fly.
#[derive(Clone, Copy)]
pub(crate) enum SourceContext {
    /// Multi-message: the line number is already known.
    Line { line: usize, column_offset: usize },
    /// Single-message / raw document: compute line+column from byte offset.
    Document,
}

impl SourceContext {
    /// Resolve a byte index inside `source` to 1-based `(line, column)`.
    ///
    /// For [`SourceContext::Line`] the `source` slice is treated as a single
    /// line and `column_offset` is added to the character column.
    /// For [`SourceContext::Document`] the full source is scanned for newlines.
    pub(crate) fn location(&self, source: &str, byte_idx: usize) -> (usize, usize) {
        match self {
            Self::Line {
                line,
                column_offset,
            } => (*line, char_column(source, byte_idx) + column_offset),
            Self::Document => byte_to_line_col(source, byte_idx),
        }
    }
}

/// Produce a debug-formatted snippet, truncated to 24 characters.
pub(crate) fn quoted_snippet(value: &str) -> String {
    let mut out = value.trim().to_string();
    if out.len() > 24 {
        out.truncate(24);
        out.push_str("...");
    }
    format!("{out:?}")
}
