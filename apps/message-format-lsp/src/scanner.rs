// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lightweight text scanner for extracting declarations, variable references,
//! and message IDs from MF2 source without requiring a full parse.
//!
//! This is intentionally tolerant of syntax errors so that navigation features
//! work in partially-valid documents.

use std::ops::Range;

/// A variable declaration found in the source.
#[derive(Debug, Clone)]
pub(crate) struct Declaration {
    /// Variable name without the `$` prefix.
    pub(crate) name: String,
    /// Whether this is `.input` or `.local`.
    pub(crate) kind: DeclarationKind,
    /// Byte range of the variable name (after `$`) in the source.
    pub(crate) name_span: Range<usize>,
    /// Byte range of the entire declaration keyword + expression.
    pub(crate) full_span: Range<usize>,
}

/// Declaration kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclarationKind {
    /// `.input {$var ...}`
    Input,
    /// `.local $var = ...`
    Local,
}

/// A reference to a `$variable` in the source.
#[derive(Debug, Clone)]
pub(crate) struct VarReference {
    /// Variable name without the `$` prefix.
    pub(crate) name: String,
    /// Byte range of the full `$name` token (including `$`).
    pub(crate) span: Range<usize>,
}

/// A message entry in catalog mode.
#[derive(Debug, Clone)]
pub(crate) struct MessageSymbol {
    /// Message ID.
    pub(crate) id: String,
    /// 0-based line number.
    #[allow(
        dead_code,
        reason = "kept for tests and future LSP symbol presentation"
    )]
    pub(crate) line: u32,
    /// Byte range of the entire line.
    pub(crate) span: Range<usize>,
}

/// Scan source text for all `$variable` references.
pub(crate) fn scan_var_references(source: &str) -> Vec<VarReference> {
    let mut refs = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i;
            i += 1; // skip '$'
            let name_start = i;
            while i < bytes.len() && is_name_char(bytes[i]) {
                i += 1;
            }
            if i > name_start {
                refs.push(VarReference {
                    name: source[name_start..i].to_string(),
                    span: start..i,
                });
            }
        } else {
            i += 1;
        }
    }
    refs
}

/// Scan source text for `.input` and `.local` declarations.
pub(crate) fn scan_declarations(source: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();

    for (line_start, line) in line_byte_ranges(source) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(".input") {
            if let Some(decl) = parse_input_declaration(rest, line_start, line, source) {
                decls.push(decl);
            }
        } else if let Some(rest) = trimmed.strip_prefix(".local") {
            if let Some(decl) = parse_local_declaration(rest, line_start, line, source) {
                decls.push(decl);
            }
        }
    }

    decls
}

/// Scan catalog-mode source for message IDs.
#[cfg(test)]
pub(crate) fn scan_message_symbols(source: &str) -> Vec<MessageSymbol> {
    let mut symbols = Vec::new();

    for (line_num, (line_start, line)) in line_byte_ranges(source).enumerate() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((id, _)) = trimmed.split_once('=') {
                let id = id.trim();
                if !id.is_empty() {
                    symbols.push(MessageSymbol {
                        id: id.to_string(),
                        line: u32::try_from(line_num).unwrap_or(u32::MAX),
                        span: line_start..line_start + line.len(),
                    });
                }
            }
        }
    }

    symbols
}

/// Parse an `.input {$var ...}` declaration.
fn parse_input_declaration(
    rest: &str,
    line_start: usize,
    line: &str,
    source: &str,
) -> Option<Declaration> {
    // Find the `$` inside the expression braces.
    let dollar = rest.find('$')?;
    let abs_dollar = line_start + (line.len() - rest.len()) + dollar;
    let name_start = abs_dollar + 1;
    let mut end = name_start;
    let bytes = source.as_bytes();
    while end < bytes.len() && is_name_char(bytes[end]) {
        end += 1;
    }
    if end == name_start {
        return None;
    }
    Some(Declaration {
        name: source[name_start..end].to_string(),
        kind: DeclarationKind::Input,
        name_span: name_start..end,
        full_span: line_start..line_start + line.len(),
    })
}

/// Parse a `.local $var = {expr}` declaration.
fn parse_local_declaration(
    rest: &str,
    line_start: usize,
    line: &str,
    source: &str,
) -> Option<Declaration> {
    let dollar = rest.find('$')?;
    let abs_dollar = line_start + (line.len() - rest.len()) + dollar;
    let name_start = abs_dollar + 1;
    let mut end = name_start;
    let bytes = source.as_bytes();
    while end < bytes.len() && is_name_char(bytes[end]) {
        end += 1;
    }
    if end == name_start {
        return None;
    }
    Some(Declaration {
        name: source[name_start..end].to_string(),
        kind: DeclarationKind::Local,
        name_span: name_start..end,
        full_span: line_start..line_start + line.len(),
    })
}

/// Iterate (`byte_offset`, `line_text`) pairs for the source.
fn line_byte_ranges(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    source.split('\n').map(move |line| {
        let start = offset;
        offset += line.len() + 1; // +1 for the '\n'
        (start, line)
    })
}

/// MF2 name characters: ASCII alphanumeric, `-`, `_`, and extended Unicode
/// identifier chars. For simplicity we accept alphanumeric + `-` + `_`.
fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_var_references() {
        let source = "Hello {$name}, you have {$count :number} items.";
        let refs = scan_var_references(source);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "name");
        assert_eq!(refs[1].name, "count");
        assert_eq!(&source[refs[0].span.clone()], "$name");
        assert_eq!(&source[refs[1].span.clone()], "$count");
    }

    #[test]
    fn scan_finds_declarations() {
        let source = ".input {$name :string}\n.local $count = {$raw :number}\nHello {$name}";
        let decls = scan_declarations(source);
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "name");
        assert_eq!(decls[0].kind, DeclarationKind::Input);
        assert_eq!(decls[1].name, "count");
        assert_eq!(decls[1].kind, DeclarationKind::Local);
    }

    #[test]
    fn scan_finds_message_symbols() {
        let source = "greeting = Hello\n# comment\nfarewell = Goodbye\n";
        let symbols = scan_message_symbols(source);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].id, "greeting");
        assert_eq!(symbols[0].line, 0);
        assert_eq!(symbols[1].id, "farewell");
        assert_eq!(symbols[1].line, 2);
    }

    #[test]
    fn scan_handles_empty_source() {
        assert!(scan_var_references("").is_empty());
        assert!(scan_declarations("").is_empty());
        assert!(scan_message_symbols("").is_empty());
    }

    #[test]
    fn scan_ignores_dollar_at_end() {
        let refs = scan_var_references("price is $");
        assert!(refs.is_empty());
    }
}
