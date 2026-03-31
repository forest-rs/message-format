// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::syntax::span::{SourceContext, byte_to_line_col};

use super::bindings::{DeclarationBindings, collect_declaration_bindings};
use super::pattern::{FunctionOriginContext, lower_pattern_node_to_parts};
use super::raw_match::lower_raw_match_ir;
use super::rewrite::lower_parts_with_declaration_bindings;

struct AnalyzedSingleMessage<'a> {
    declarations: crate::syntax::semantic::CanonicalDeclarationPrelude<'a>,
    bindings: DeclarationBindings,
    doc: crate::syntax::ast::SyntaxDocument<'a>,
}

pub(super) fn parse_single_message(
    source: &str,
    options: CompileOptions,
    source_id: Option<SourceId>,
) -> Result<Message, CompileError> {
    let ctx = SourceContext::Document;
    let analyzed = analyze_single_message(source, ctx, source_id)?;
    if let Some(match_prelude) =
        crate::syntax::semantic::parse_match_declaration_prelude_from_document(
            source,
            &analyzed.doc,
            ctx,
        )?
    {
        return Ok(Message {
            id: String::from("main"),
            parts: lower_raw_match_ir(
                source,
                ctx,
                &analyzed.bindings,
                match_prelude,
                options,
                source_id.map(|source_id| FunctionOriginContext {
                    source_id,
                    base_byte: 0,
                }),
            )?,
            origin: source_id.map(|source_id| SourceSpan {
                source_id,
                byte_start: 0,
                byte_end: source.len(),
                line: Some(1),
                column: Some(1),
            }),
            message_source: Some(source.to_string()),
            upstream_origin: None,
        });
    }

    Ok(Message {
        id: String::from("main"),
        parts: preprocess_single_message_parts(source, ctx, analyzed, options, source_id)?,
        origin: source_id.map(|source_id| SourceSpan {
            source_id,
            byte_start: 0,
            byte_end: source.len(),
            line: Some(1),
            column: Some(1),
        }),
        message_source: Some(source.to_string()),
        upstream_origin: None,
    })
}

pub(super) fn validate_and_parse_declarations(
    source: &str,
    ctx: SourceContext,
) -> Result<crate::syntax::semantic::CanonicalDeclarationPrelude<'_>, CompileError> {
    crate::syntax::parser::validate_known_declaration_heads(source, ctx)?;
    let declarations = crate::syntax::semantic::parse_declaration_prelude(source, ctx)?;
    crate::syntax::semantic::validate_declaration_scope(source, &declarations, ctx)?;
    crate::syntax::semantic::validate_no_implicit_redeclaration(source, ctx)?;
    crate::syntax::semantic::canonicalize_declaration_prelude(source, declarations, ctx)
}

fn analyze_single_message(
    source: &str,
    ctx: SourceContext,
    source_id: Option<SourceId>,
) -> Result<AnalyzedSingleMessage<'_>, CompileError> {
    let declarations = validate_and_parse_declarations(source, ctx)?;
    let bindings = collect_declaration_bindings(
        source,
        &declarations,
        ctx,
        source_id.map(|source_id| FunctionOriginContext {
            source_id,
            base_byte: 0,
        }),
    )?;
    let doc = crate::syntax::parser::parse_document(source);
    Ok(AnalyzedSingleMessage {
        declarations,
        bindings,
        doc,
    })
}

fn preprocess_single_message_parts(
    source: &str,
    ctx: SourceContext,
    analyzed: AnalyzedSingleMessage<'_>,
    options: CompileOptions,
    source_id: Option<SourceId>,
) -> Result<Vec<Part>, CompileError> {
    let (_, body) = split_declarations_and_body_from_doc(source, &analyzed.doc);
    let (line, _) = ctx.location(source, 0);
    ensure_well_formed_quoted_pattern_body(body, line)?;

    let quoted_pattern = extract_quoted_pattern(body);
    let has_declarations =
        !analyzed.declarations.inputs.is_empty() || !analyzed.declarations.locals.is_empty();
    if has_declarations && quoted_pattern.is_none() {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::invalid_expr_detail(
            line,
            col,
            "message body '{{...}}' after declarations",
            "end of input",
        ));
    }

    let pattern = quoted_pattern.unwrap_or(body);
    let pattern_ctx = pattern_context_for_subslice(source, pattern, ctx);
    let pattern_node = crate::syntax::parser::parse_pattern(pattern);
    let mut parts = lower_pattern_node_to_parts(
        pattern,
        &pattern_node,
        pattern_ctx,
        options,
        source_id.map(|source_id| FunctionOriginContext {
            source_id,
            base_byte: pattern.as_ptr() as usize - source.as_ptr() as usize,
        }),
    )?;
    lower_parts_with_declaration_bindings(&mut parts, &analyzed.bindings, has_declarations)?;
    Ok(parts)
}

fn pattern_context_for_subslice(source: &str, pattern: &str, ctx: SourceContext) -> SourceContext {
    match ctx {
        SourceContext::Document => {
            let pattern_byte_start = pattern.as_ptr() as usize - source.as_ptr() as usize;
            let (line, col) = byte_to_line_col(source, pattern_byte_start);
            SourceContext::Line {
                line,
                column_offset: col.saturating_sub(1),
            }
        }
        other => other,
    }
}

fn split_declarations_and_body_from_doc<'a>(
    source: &'a str,
    doc: &crate::syntax::ast::SyntaxDocument<'_>,
) -> (&'a str, &'a str) {
    if let (Some(first_decl), Some(last_decl)) = (doc.declarations.first(), doc.declarations.last())
    {
        let body_start = doc
            .body
            .as_ref()
            .map_or(last_decl.span.end, |body| body.span.start);
        return (
            &source[first_decl.span.start..last_decl.span.end],
            &source[body_start..],
        );
    }
    ("", source)
}

fn extract_quoted_pattern(source: &str) -> Option<&str> {
    let start = source.find("{{")?;
    let end = source.rfind("}}")?;
    if end < start + 2 {
        return None;
    }
    Some(&source[start + 2..end])
}
