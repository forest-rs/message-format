// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Navigation request handlers: go-to-definition, find references, document symbols.

use lsp_types::{
    DocumentSymbol, DocumentSymbolResponse, GotoDefinitionResponse, Location, Position, Range,
    SymbolKind, Url,
};

use crate::analysis::DocumentAnalysis;

/// Convert a byte offset in `source` to an LSP `Position`.
fn offset_to_position(source: &str, offset: usize) -> Position {
    let mut line = 0_u32;
    let mut col = 0_u32;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Position::new(line, col)
}

/// Convert a byte range in `source` to an LSP `Range`.
fn span_to_range(source: &str, span: &std::ops::Range<usize>) -> Range {
    Range::new(
        offset_to_position(source, span.start),
        offset_to_position(source, span.end),
    )
}

/// Find the variable name at the given cursor position, if any.
fn var_at_position(source: &str, analysis: &DocumentAnalysis, pos: Position) -> Option<String> {
    let offset = position_to_offset(source, pos)?;

    // Check declarations.
    for decl in &analysis.declarations {
        // Match the `$name` range (dollar sign is one byte before name_span.start).
        let dollar_start = decl.name_span.start.saturating_sub(1);
        if offset >= dollar_start && offset < decl.name_span.end {
            return Some(decl.name.clone());
        }
    }

    // Check references.
    for r in &analysis.var_references {
        if offset >= r.span.start && offset < r.span.end {
            return Some(r.name.clone());
        }
    }

    None
}

/// Convert an LSP `Position` to a byte offset in `source`.
fn position_to_offset(source: &str, pos: Position) -> Option<usize> {
    let mut line = 0_u32;
    let mut col = 0_u32;
    for (i, ch) in source.char_indices() {
        if line == pos.line && col == pos.character {
            return Some(i);
        }
        if ch == '\n' {
            if line == pos.line {
                // Position is past end of this line; clamp to newline.
                return Some(i);
            }
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    // Position at end of file.
    if line == pos.line && col == pos.character {
        return Some(source.len());
    }
    None
}

/// Handle `textDocument/definition`.
pub(crate) fn goto_definition(
    source: &str,
    uri: &Url,
    analysis: &DocumentAnalysis,
    pos: Position,
) -> Option<GotoDefinitionResponse> {
    let name = var_at_position(source, analysis, pos)?;

    // Look for a declaration of this variable.
    for decl in &analysis.declarations {
        if decl.name == name {
            let range = span_to_range(source, &decl.name_span);
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range,
            }));
        }
    }

    None
}

/// Handle `textDocument/references`.
pub(crate) fn find_references(
    source: &str,
    uri: &Url,
    analysis: &DocumentAnalysis,
    pos: Position,
) -> Option<Vec<Location>> {
    let name = var_at_position(source, analysis, pos)?;

    let mut locations = Vec::new();

    // Include the declaration site(s).
    for decl in &analysis.declarations {
        if decl.name == name {
            let dollar_start = decl.name_span.start.saturating_sub(1);
            let range = span_to_range(source, &(dollar_start..decl.name_span.end));
            locations.push(Location {
                uri: uri.clone(),
                range,
            });
        }
    }

    // Include all references.
    for r in &analysis.var_references {
        if r.name == name {
            let range = span_to_range(source, &r.span);
            locations.push(Location {
                uri: uri.clone(),
                range,
            });
        }
    }

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// Handle `textDocument/documentSymbol`.
pub(crate) fn document_symbols(
    source: &str,
    analysis: &DocumentAnalysis,
) -> DocumentSymbolResponse {
    let mut symbols = Vec::new();

    if analysis.kind != crate::analysis::DocumentKind::SingleMessage {
        // In catalog mode, each message ID is a symbol.
        for msg in &analysis.message_symbols {
            let range = span_to_range(source, &msg.span);
            #[allow(deprecated, reason = "DocumentSymbol requires deprecated field")]
            symbols.push(DocumentSymbol {
                name: msg.id.clone(),
                detail: None,
                kind: SymbolKind::STRING,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
    } else {
        // In single-message mode, show declarations as symbols.
        for decl in &analysis.declarations {
            let selection_range = span_to_range(source, &decl.name_span);
            let full_range = span_to_range(source, &decl.full_span);
            #[allow(deprecated, reason = "DocumentSymbol requires deprecated field")]
            symbols.push(DocumentSymbol {
                name: format!("${}", decl.name),
                detail: Some(format!("{:?}", decl.kind).to_lowercase()),
                kind: SymbolKind::VARIABLE,
                tags: None,
                deprecated: None,
                range: full_range,
                selection_range,
                children: None,
            });
        }
    }

    DocumentSymbolResponse::Nested(symbols)
}
