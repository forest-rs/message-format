// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Semantic checks for parsed syntax scaffolding.

use core::ops::Range;

use icu_normalizer::ComposingNormalizer;

use crate::compile::CompileError;
use crate::semantic::{FunctionOption, FunctionOptionValue, FunctionSpec};
use crate::syntax::literal::parse_literal_text;
use crate::syntax::parser::parse_document;
use crate::syntax::span::{SourceContext, byte_to_line_col, quoted_snippet};
use crate::syntax::{
    ast::{DeclarationKind, DeclarationNode, DeclarationPayloadNode},
    ident::{
        canonicalize_identifier, is_bidi_control, is_identifier, is_ignorable_char,
        strip_bidi_controls, strip_boundary_bidi_controls,
    },
};

/// Parsed `.input` declaration payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputDeclaration<'a> {
    pub(crate) expr: SemanticExpression<'a>,
}

/// Parsed `.local` declaration payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalDeclaration<'a> {
    pub(crate) name: String,
    pub(crate) expr: SemanticExpression<'a>,
}

/// Parsed declaration prelude payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclarationPrelude<'a> {
    pub(crate) inputs: Vec<InputDeclaration<'a>>,
    pub(crate) locals: Vec<LocalDeclaration<'a>>,
}

/// Typed semantic expression payload extracted from syntax AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticExpression<'a> {
    pub(crate) node: crate::syntax::ast::ExpressionNode<'a>,
}

/// Canonicalized `.input` declaration payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalInputDeclaration<'a> {
    pub(crate) canonical: String,
    pub(crate) source_name: String,
    pub(crate) expr: SemanticExpression<'a>,
}

/// Canonicalized `.local` declaration payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalLocalDeclaration<'a> {
    pub(crate) canonical: String,
    pub(crate) expr: SemanticExpression<'a>,
}

/// Canonicalized declaration prelude payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalDeclarationPrelude<'a> {
    pub(crate) inputs: Vec<CanonicalInputDeclaration<'a>>,
    pub(crate) locals: Vec<CanonicalLocalDeclaration<'a>>,
}

/// Parsed `.match` declaration prelude payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchDeclarationPrelude<'a> {
    pub(crate) selectors: Vec<MatchSelectorPrelude>,
    pub(crate) variants: Vec<MatchVariantPrelude<'a>>,
    pub(crate) duplicate_keys: Vec<VariantKeyOccurrence>,
}

/// Parsed and canonicalized `.match` selector payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchSelectorPrelude {
    pub(crate) span: Range<usize>,
    pub(crate) canonical: String,
}

/// Parsed `.match` variant payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchVariantPrelude<'a> {
    pub(crate) keys: Vec<MatchVariantKeyPrelude>,
    pub(crate) pattern: crate::syntax::ast::PatternNode<'a>,
}

/// Parsed `.match` variant key payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchVariantKeyPrelude {
    pub(crate) span: Range<usize>,
    pub(crate) is_default: bool,
    pub(crate) key: String,
}

/// Single-selector variant-key occurrence metadata for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VariantKeyOccurrence {
    pub(crate) key: String,
    pub(crate) is_default: bool,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) display: String,
}

/// Parse a variable token like `$name` into a validated identifier.
pub(crate) fn parse_prefixed_variable_token(
    value: &str,
    line: usize,
    column: usize,
) -> Result<String, CompileError> {
    let trimmed = value.trim();
    if !trimmed.starts_with('$') {
        return Err(CompileError::invalid_expr_detail(
            line,
            column,
            "variable starting with '$'",
            quoted_snippet(trimmed),
        ));
    }
    let ident = strip_boundary_bidi_controls(trimmed[1..].trim()).ok_or_else(|| {
        CompileError::invalid_var_detail(
            line,
            column.saturating_add(1),
            "valid identifier after '$'",
            quoted_snippet(trimmed),
        )
    })?;
    if !is_identifier(&canonicalize_identifier(&ident)) {
        return Err(CompileError::invalid_var_detail(
            line,
            column.saturating_add(1),
            "valid identifier after '$'",
            quoted_snippet(trimmed),
        ));
    }
    Ok(ident)
}

/// Parse a function annotation and options from AST function node spans.
pub(crate) fn parse_function_spec_node(
    source: &str,
    function: &crate::syntax::ast::FunctionSpecNode<'_>,
    ctx: SourceContext,
) -> Result<FunctionSpec, CompileError> {
    let (line, column) = ctx.location(source, function.name_span.start);
    let func = function.name.trim();
    if func.is_empty() {
        return Err(CompileError::invalid_expr_detail(
            line,
            column,
            "function name after ':'",
            "end of expression",
        ));
    }
    if !is_function_name(func) {
        return Err(CompileError::invalid_expr_detail(
            line,
            column,
            "function identifier",
            quoted_snippet(func),
        ));
    }
    let option_tokens = function
        .options
        .iter()
        .map(|token| token.trim_matches(is_ignorable_char))
        .collect::<Vec<_>>();
    let options = parse_options_tail(func, option_tokens, line, column)?;
    Ok(FunctionSpec {
        name: func.to_string(),
        options,
        origin: None,
    })
}

/// Validate that single-selector variant keys are unique under NFC normalization.
pub(crate) fn validate_no_duplicate_single_selector_keys(
    keys: &[VariantKeyOccurrence],
) -> Result<(), CompileError> {
    let normalizer = ComposingNormalizer::new_nfc();
    let mut first: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();
    for occurrence in keys {
        if occurrence.is_default {
            continue;
        }
        let normalized = normalizer.normalize(&occurrence.key).to_string();
        if let Some(previous) = first.get(&normalized) {
            return Err(CompileError::duplicate_variant_detail(
                occurrence.line,
                occurrence.column,
                "unique variant key (NFC-normalized)",
                format!(
                    "{} duplicates key first seen at {}:{}",
                    quoted_snippet(&occurrence.display),
                    previous.0,
                    previous.1
                ),
            ));
        }
        first.insert(normalized, (occurrence.line, occurrence.column));
    }
    Ok(())
}

/// Validate that multi-selector variant key tuples are unique.
fn validate_no_duplicate_variant_tuples(
    variants: &[MatchVariantPrelude<'_>],
    ctx: SourceContext,
    source: &str,
) -> Result<(), CompileError> {
    let mut seen: std::collections::BTreeMap<Vec<&str>, usize> = std::collections::BTreeMap::new();
    for (idx, variant) in variants.iter().enumerate() {
        let tuple: Vec<&str> = variant.keys.iter().map(|k| k.key.as_str()).collect();
        if let Some(&prev_idx) = seen.get(&tuple) {
            let display: Vec<_> = variant.keys.iter().map(|k| k.key.as_str()).collect();
            let prev_display: Vec<_> = variants[prev_idx]
                .keys
                .iter()
                .map(|k| k.key.as_str())
                .collect();
            let (line, col) =
                ctx.location(source, variant.keys.first().map_or(0, |k| k.span.start));
            return Err(CompileError::duplicate_variant_detail(
                line,
                col,
                "unique variant key tuple",
                format!(
                    "variant [{}] duplicates [{}]",
                    display.join(" "),
                    prev_display.join(" "),
                ),
            ));
        }
        seen.insert(tuple, idx);
    }
    Ok(())
}

/// Parse all declaration payloads from the declaration prelude.
pub(crate) fn parse_declaration_prelude(
    source: &str,
    ctx: SourceContext,
) -> Result<DeclarationPrelude<'_>, CompileError> {
    let doc = parse_document(source);
    parse_declaration_prelude_from_document(source, &doc, ctx)
}

/// Parse declaration payloads from an existing syntax document.
pub(crate) fn parse_declaration_prelude_from_document<'a>(
    source: &'a str,
    doc: &crate::syntax::ast::SyntaxDocument<'a>,
    ctx: SourceContext,
) -> Result<DeclarationPrelude<'a>, CompileError> {
    let mut inputs = Vec::new();
    let mut locals = Vec::new();
    for declaration in &doc.declarations {
        match declaration.kind {
            DeclarationKind::Input => {
                inputs.push(parse_input_declaration(source, declaration)?);
            }
            DeclarationKind::Local => {
                locals.push(parse_local_declaration(source, declaration, ctx)?);
            }
            DeclarationKind::Match => {}
        }
    }
    Ok(DeclarationPrelude { inputs, locals })
}

/// Validate declaration prelude scoping and ordering constraints.
pub(crate) fn validate_declaration_scope(
    source: &str,
    declarations: &DeclarationPrelude<'_>,
    ctx: SourceContext,
) -> Result<(), CompileError> {
    let mut declared = std::collections::BTreeSet::new();
    let mut inputs = std::collections::BTreeSet::new();
    let all_locals = collect_local_names(&declarations.locals);

    for declaration in &declarations.inputs {
        let var = parse_input_declared_var(source, &declaration.expr.node, ctx)?;
        if declared.contains(&var) {
            let (line, col) = ctx.location(source, declaration.expr.node.span.start);
            return Err(CompileError::duplicate_declaration_detail(
                line,
                col,
                "unique declaration",
                format!("${} already declared", var),
            ));
        }
        declared.insert(var.clone());
        inputs.insert(var);
    }

    let mut available = inputs;
    for declaration in &declarations.locals {
        let (line, col) = ctx.location(source, declaration.expr.node.span.start);
        let Some(name) = strip_boundary_bidi_controls(&declaration.name) else {
            return Err(CompileError::invalid_expr_detail(
                line,
                col,
                "valid local variable name",
                quoted_snippet(&declaration.name),
            ));
        };
        let name = canonicalize_identifier(&name);
        if !is_identifier(&name) {
            return Err(CompileError::invalid_expr_detail(
                line,
                col,
                "valid local variable name",
                quoted_snippet(&declaration.name),
            ));
        }
        if declared.contains(&name) {
            return Err(CompileError::duplicate_declaration_detail(
                line,
                col,
                "unique declaration",
                format!("${} already declared", name),
            ));
        }

        for reference in collect_var_refs(&declaration.expr.node) {
            if reference == name {
                return Err(CompileError::duplicate_declaration_detail(
                    line,
                    col,
                    "no self-reference in .local",
                    format!("${} references itself", name),
                ));
            }
            if all_locals.contains(&reference) && !available.contains(&reference) {
                return Err(CompileError::duplicate_declaration_detail(
                    line,
                    col,
                    "declared variable reference",
                    format!("${} not yet available", reference),
                ));
            }
        }

        declared.insert(name.clone());
        available.insert(name);
    }

    Ok(())
}

/// Validate that no implicitly-declared variable is later explicitly declared (TR35 S-13).
///
/// A variable is "implicitly declared" when it first appears as a reference in
/// a `.local` expression.  If a subsequent `.input` then explicitly declares
/// the same variable, that is a data-model error.
pub(crate) fn validate_no_implicit_redeclaration(
    source: &str,
    ctx: SourceContext,
) -> Result<(), CompileError> {
    let doc = parse_document(source);
    let mut explicitly_declared = std::collections::BTreeSet::new();
    let mut implicitly_referenced = std::collections::BTreeSet::new();

    for declaration in &doc.declarations {
        match declaration.kind {
            DeclarationKind::Input => {
                let Some(DeclarationPayloadNode::Input { expr }) = &declaration.payload else {
                    continue;
                };
                let Ok(var) = parse_input_declared_var(source, expr, ctx) else {
                    continue;
                };
                if implicitly_referenced.contains(&var) {
                    let (line, col) = ctx.location(source, declaration.span.start);
                    return Err(CompileError::duplicate_declaration_detail(
                        line,
                        col,
                        "no prior implicit reference",
                        format!(
                            "${} is implicitly declared by prior use and cannot be re-declared with .input",
                            var
                        ),
                    ));
                }
                explicitly_declared.insert(var);
            }
            DeclarationKind::Local => {
                let Some(DeclarationPayloadNode::Local { expr, name, .. }) = &declaration.payload
                else {
                    continue;
                };
                // The local's own name is explicitly declared by this .local, not implicitly.
                let local_name =
                    strip_boundary_bidi_controls(name).map(|n| canonicalize_identifier(&n));

                let refs = collect_var_refs(expr);
                for reference in refs {
                    if local_name.as_ref() != Some(&reference)
                        && !explicitly_declared.contains(&reference)
                    {
                        implicitly_referenced.insert(reference);
                    }
                }
                if let Some(name) = local_name {
                    explicitly_declared.insert(name);
                }
            }
            DeclarationKind::Match => {}
        }
    }
    Ok(())
}

/// Build canonical declaration model after scope validation.
pub(crate) fn canonicalize_declaration_prelude<'a>(
    source: &'a str,
    declarations: DeclarationPrelude<'a>,
    ctx: SourceContext,
) -> Result<CanonicalDeclarationPrelude<'a>, CompileError> {
    let mut inputs = Vec::with_capacity(declarations.inputs.len());
    for declaration in declarations.inputs {
        let source_name = parse_input_declared_source_name(source, &declaration.expr.node, ctx)?;
        let canonical = canonicalize_identifier(&source_name);
        inputs.push(CanonicalInputDeclaration {
            canonical,
            source_name,
            expr: declaration.expr,
        });
    }

    let mut locals = Vec::with_capacity(declarations.locals.len());
    for declaration in declarations.locals {
        let (line, col) = ctx.location(source, declaration.expr.node.span.start);
        let Some(name) = strip_boundary_bidi_controls(&declaration.name) else {
            return Err(CompileError::invalid_expr_detail(
                line,
                col,
                "valid local variable name",
                quoted_snippet(&declaration.name),
            ));
        };
        let canonical = canonicalize_identifier(&name);
        if !is_identifier(&canonical) {
            return Err(CompileError::invalid_expr_detail(
                line,
                col,
                "valid local variable name",
                quoted_snippet(&declaration.name),
            ));
        }
        locals.push(CanonicalLocalDeclaration {
            canonical,
            expr: declaration.expr,
        });
    }

    Ok(CanonicalDeclarationPrelude { inputs, locals })
}

/// Parse `.match` declaration prelude payload from source, if present.
#[cfg(test)]
pub(crate) fn parse_match_declaration_prelude(
    source: &str,
    ctx: SourceContext,
) -> Result<Option<MatchDeclarationPrelude<'_>>, CompileError> {
    let doc = parse_document(source);
    parse_match_declaration_prelude_from_document(source, &doc, ctx)
}

/// Parse `.match` declaration prelude payload from an existing syntax document, if present.
pub(crate) fn parse_match_declaration_prelude_from_document<'a>(
    source: &'a str,
    doc: &crate::syntax::ast::SyntaxDocument<'a>,
    ctx: SourceContext,
) -> Result<Option<MatchDeclarationPrelude<'a>>, CompileError> {
    let Some(declaration) = doc
        .declarations
        .iter()
        .find(|declaration| declaration.kind == DeclarationKind::Match)
    else {
        return Ok(None);
    };

    let Some(DeclarationPayloadNode::Match {
        selectors,
        variants,
    }) = &declaration.payload
    else {
        let match_head_end = declaration.span.start + ".match".len();
        if let Some(ch) = source[match_head_end..].chars().next()
            && !(crate::syntax::charset::is_mf2_whitespace(ch) || is_bidi_control(ch))
        {
            let (line, column) = ctx.location(source, match_head_end);
            return Err(CompileError::invalid_expr_detail(
                line,
                column,
                "whitespace after '.match'",
                format!("{ch:?}"),
            ));
        }
        let (line, column) = ctx.location(source, declaration.span.start);
        return Err(CompileError::invalid_expr_detail(
            line,
            column,
            "at least one selector variable",
            "none",
        ));
    };

    let mut parsed_selectors = Vec::with_capacity(selectors.len());
    for selector in selectors {
        let Some(crate::syntax::ast::ExpressionPayloadNode::Var(var)) = &selector.payload else {
            let (line, column) = ctx.location(source, selector.span.start);
            return Err(CompileError::invalid_var_detail(
                line,
                column,
                "variable selector after '.match'",
                quoted_snippet(source.get(selector.span.clone()).ok_or_else(|| {
                    let (l, _) = ctx.location(source, 0);
                    CompileError::invalid_expr(l)
                })?),
            ));
        };

        let raw_name = var.name;
        let Some(name) = strip_boundary_bidi_controls(raw_name) else {
            let (line, column) = ctx.location(source, selector.span.start);
            return Err(CompileError::invalid_var_detail(
                line,
                column,
                "valid identifier after '$'",
                quoted_snippet(raw_name),
            ));
        };
        let canonical = canonicalize_identifier(&name);
        if !is_identifier(&canonical) {
            let (line, column) = ctx.location(source, selector.span.start);
            return Err(CompileError::invalid_var_detail(
                line,
                column,
                "valid identifier after '$'",
                quoted_snippet(raw_name),
            ));
        }
        parsed_selectors.push(MatchSelectorPrelude {
            span: selector.span.clone(),
            canonical,
        });
    }

    let mut duplicate_keys = Vec::new();
    let variants = variants
        .iter()
        .map(|variant| {
            if variant.keys.len() != parsed_selectors.len() {
                let (line, column) = ctx.location(source, variant.pattern.span.start);
                return Err(CompileError::variant_key_mismatch_detail(
                    line,
                    column,
                    "match key for each selector",
                    "mismatched key count",
                ));
            }
            let keys = variant
                .keys
                .iter()
                .map(|key| {
                    if key.is_default {
                        return Ok(MatchVariantKeyPrelude {
                            span: key.span.clone(),
                            is_default: true,
                            key: String::from("other"),
                        });
                    }
                    let raw = key.text;
                    let (line, _) = ctx.location(source, key.span.start);
                    let decoded = if key.is_quoted {
                        decode_quoted_literal(raw, line)?
                    } else {
                        strip_bidi_controls(raw.trim())
                    };
                    // TR35: "The resolved value of each key MUST be in
                    // Unicode Normalization Form C."
                    let normalizer = ComposingNormalizer::new_nfc();
                    let decoded = normalizer.normalize(&decoded).to_string();
                    if parsed_selectors.len() == 1 {
                        let (line, column) = ctx.location(source, key.span.start);
                        duplicate_keys.push(VariantKeyOccurrence {
                            key: decoded.clone(),
                            is_default: false,
                            line,
                            column,
                            display: raw.to_string(),
                        });
                    }
                    Ok(MatchVariantKeyPrelude {
                        span: key.span.clone(),
                        is_default: false,
                        key: decoded,
                    })
                })
                .collect::<Result<Vec<_>, CompileError>>()?;
            Ok(MatchVariantPrelude {
                keys,
                pattern: variant.pattern.clone(),
            })
        })
        .collect::<Result<Vec<_>, CompileError>>()?;

    // TR35: detect duplicate variant key tuples (works for any selector count).
    if parsed_selectors.len() > 1 {
        validate_no_duplicate_variant_tuples(&variants, ctx, source)?;
    }

    Ok(Some(MatchDeclarationPrelude {
        selectors: parsed_selectors,
        variants,
        duplicate_keys,
    }))
}

fn parse_input_declaration<'a>(
    source: &'a str,
    declaration: &DeclarationNode<'a>,
) -> Result<InputDeclaration<'a>, CompileError> {
    let Some(DeclarationPayloadNode::Input { expr }) = &declaration.payload else {
        return Err(declaration_payload_error(
            source,
            declaration,
            ".input {<expression>}",
        ));
    };
    Ok(InputDeclaration {
        expr: SemanticExpression { node: expr.clone() },
    })
}

fn parse_local_declaration<'a>(
    source: &'a str,
    declaration: &DeclarationNode<'a>,
    ctx: SourceContext,
) -> Result<LocalDeclaration<'a>, CompileError> {
    let Some(DeclarationPayloadNode::Local { name, expr, .. }) = &declaration.payload else {
        return Err(declaration_payload_error(
            source,
            declaration,
            ".local $name = {<expression>}",
        ));
    };
    if name.is_empty() {
        let (line, col) = ctx.location(source, declaration.span.start);
        return Err(CompileError::invalid_expr_detail(
            line,
            col,
            "variable name after .local",
            "empty name",
        ));
    }
    Ok(LocalDeclaration {
        name: name.to_string(),
        expr: SemanticExpression { node: expr.clone() },
    })
}

fn parse_input_declared_source_name(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
) -> Result<String, CompileError> {
    let err = || {
        let (line, col) = ctx.location(source, expr.span.start);
        CompileError::invalid_expr_detail(
            line,
            col,
            "variable in .input expression",
            quoted_snippet(source.get(expr.raw_span.clone()).unwrap_or("")),
        )
    };
    let var = match &expr.payload {
        Some(crate::syntax::ast::ExpressionPayloadNode::Var(var)) => var,
        Some(crate::syntax::ast::ExpressionPayloadNode::Call(call)) => match &call.operand {
            crate::syntax::ast::CallOperandNode::Var(var) => var,
            crate::syntax::ast::CallOperandNode::Literal { .. } => {
                return Err(err());
            }
        },
        _ => return Err(err()),
    };
    let raw = var.name;
    let Some(value) = strip_boundary_bidi_controls(raw) else {
        return Err(err());
    };
    if !is_identifier(&canonicalize_identifier(&value)) {
        return Err(err());
    };
    let trimmed = raw.trim();
    let start = trimmed
        .char_indices()
        .find(|(_, ch)| !is_bidi_control(*ch) && !crate::syntax::charset::is_mf2_whitespace(*ch))
        .map_or(0, |(idx, _)| idx);
    let end = trimmed
        .char_indices()
        .rfind(|(_, ch)| !is_bidi_control(*ch) && !crate::syntax::charset::is_mf2_whitespace(*ch))
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    if end < start {
        return Err(err());
    }
    Ok(trimmed[start..end].to_string())
}

fn parse_input_declared_var(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
) -> Result<String, CompileError> {
    let source_name = parse_input_declared_source_name(source, expr, ctx)?;
    let canonical = canonicalize_identifier(&source_name);
    if !is_identifier(&canonical) {
        let (line, col) = ctx.location(source, expr.span.start);
        return Err(CompileError::invalid_expr_detail(
            line,
            col,
            "valid identifier",
            quoted_snippet(source.get(expr.raw_span.clone()).unwrap_or("")),
        ));
    }
    Ok(canonical)
}

fn collect_local_names(locals: &[LocalDeclaration<'_>]) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for declaration in locals {
        let Some(name) = strip_boundary_bidi_controls(&declaration.name) else {
            continue;
        };
        let canonical = canonicalize_identifier(&name);
        if is_identifier(&canonical) {
            names.insert(canonical);
        }
    }
    names
}

fn collect_var_refs(expr: &crate::syntax::ast::ExpressionNode<'_>) -> Vec<String> {
    let mut refs = Vec::new();
    collect_var_refs_from_node(expr, &mut refs);
    refs
}

fn collect_var_refs_from_node(
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    out: &mut Vec<String>,
) {
    let Some(payload) = &expr.payload else {
        return;
    };
    match payload {
        crate::syntax::ast::ExpressionPayloadNode::Var(var) => {
            collect_var_ref(var.name, out);
        }
        crate::syntax::ast::ExpressionPayloadNode::Call(call) => {
            if let crate::syntax::ast::CallOperandNode::Var(var) = &call.operand {
                collect_var_ref(var.name, out);
            }
            collect_var_refs_from_function_options(&call.function.options, out);
        }
        crate::syntax::ast::ExpressionPayloadNode::Literal(literal) => {
            if let Some(function) = &literal.function {
                collect_var_refs_from_function_options(&function.options, out);
            }
        }
        crate::syntax::ast::ExpressionPayloadNode::Markup(_) => {}
    }
}

fn collect_var_ref(raw: &str, out: &mut Vec<String>) {
    let Some(value) = strip_boundary_bidi_controls(raw) else {
        return;
    };
    let canonical = canonicalize_identifier(&value);
    if is_identifier(&canonical) {
        out.push(canonical);
    }
}

fn collect_var_refs_from_function_options(options: &[&str], out: &mut Vec<String>) {
    for token in options.iter().map(|it| it.trim()) {
        let Some((_, value)) = token.split_once('=') else {
            continue;
        };
        let Some(var) = value.strip_prefix('$') else {
            continue;
        };
        let Some(value) = strip_boundary_bidi_controls(var.trim()) else {
            continue;
        };
        let canonical = canonicalize_identifier(&value);
        if is_identifier(&canonical) {
            out.push(canonical);
        }
    }
}

/// ABNF `escaped-char` in quoted literals: `backslash ( backslash / "{" / "|" / "}" )`
fn decode_quoted_literal(value: &str, line: usize) -> Result<String, CompileError> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let Some(next) = chars.next() else {
                return Err(CompileError::invalid_expr(line));
            };
            if !matches!(next, '\\' | '{' | '|' | '}') {
                return Err(CompileError::invalid_expr(line));
            }
            out.push(next);
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_options_tail(
    _func: &str,
    tokens: Vec<&str>,
    line: usize,
    column: usize,
) -> Result<Vec<FunctionOption>, CompileError> {
    let mut filtered = Vec::new();
    let mut seen_keys = std::collections::BTreeSet::new();
    for token in &tokens {
        if token.starts_with('@') {
            continue;
        }
        let Some((key, value)) = token.split_once('=') else {
            return Err(CompileError::invalid_expr_detail(
                line,
                column,
                "option token 'key=value' or attribute '@name'",
                quoted_snippet(token),
            ));
        };
        if key.is_empty() || value.is_empty() {
            return Err(CompileError::invalid_expr_detail(
                line,
                column,
                "option token 'key=value'",
                quoted_snippet(token),
            ));
        }
        if !seen_keys.insert(key) {
            return Err(CompileError::duplicate_option_name_detail(
                line,
                column,
                "unique option names",
                format!("duplicate option {}", quoted_snippet(key)),
            ));
        }
        filtered.push(make_function_option(key, value, line, column)?);
    }
    Ok(filtered)
}

fn make_function_option(
    key: &str,
    raw_value: &str,
    line: usize,
    column: usize,
) -> Result<FunctionOption, CompileError> {
    let value = if let Some(var_tail) = raw_value.strip_prefix('$') {
        let mut prefixed = String::from("$");
        prefixed.push_str(var_tail);
        let var = parse_prefixed_variable_token(&prefixed, line, column)?;
        FunctionOptionValue::Var(canonicalize_identifier(&var))
    } else {
        FunctionOptionValue::Literal(parse_literal_text(raw_value, line)?)
    };
    Ok(FunctionOption {
        key: key.to_string(),
        value,
    })
}

fn is_function_name(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    value.split(':').all(|segment| {
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() {
            return false;
        }
        chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    })
}

fn declaration_payload_error(
    source: &str,
    declaration: &DeclarationNode<'_>,
    expected: &'static str,
) -> CompileError {
    let (line, column) = byte_to_line_col(source, declaration.span.start);
    let found = source
        .get(declaration.span.clone())
        .map_or_else(|| String::from("end of input"), quoted_snippet);
    CompileError::invalid_expr_detail(line, column, expected, found)
}

#[cfg(test)]
mod tests {
    use super::{
        VariantKeyOccurrence, parse_declaration_prelude, parse_function_spec_node,
        parse_match_declaration_prelude, parse_prefixed_variable_token,
        validate_no_duplicate_single_selector_keys,
    };
    use crate::syntax::ast::ExpressionPayloadNode;
    use crate::syntax::span::SourceContext;

    fn line_ctx(line: usize) -> SourceContext {
        SourceContext::Line {
            line,
            column_offset: 0,
        }
    }

    #[test]
    fn parses_input_declarations() {
        let src = ".input {$x :number} .input {$y}";
        let inputs = parse_declaration_prelude(src, line_ctx(1))
            .expect("parsed")
            .inputs;
        assert_eq!(inputs.len(), 2);
        assert_eq!(&src[inputs[0].expr.node.span.clone()], "$x :number");
        assert_eq!(&src[inputs[1].expr.node.span.clone()], "$y");
        assert!(matches!(
            inputs[0].expr.node.payload,
            Some(ExpressionPayloadNode::Call(_))
        ));
        assert!(matches!(
            inputs[1].expr.node.payload,
            Some(ExpressionPayloadNode::Var(_))
        ));
    }

    #[test]
    fn parses_local_declarations() {
        let src = ".local $x = {1} .local $y = {$x :number}";
        let locals = parse_declaration_prelude(src, line_ctx(1))
            .expect("parsed")
            .locals;
        assert_eq!(locals.len(), 2);
        assert_eq!(locals[0].name, "x");
        assert_eq!(&src[locals[0].expr.node.span.clone()], "1");
        assert_eq!(locals[1].name, "y");
        assert_eq!(&src[locals[1].expr.node.span.clone()], "$x :number");
    }

    #[test]
    fn parses_declaration_prelude_once() {
        let src = ".input {$x :number} .local $y = {$x}";
        let prelude = parse_declaration_prelude(src, line_ctx(1)).expect("parsed");
        assert_eq!(prelude.inputs.len(), 1);
        assert_eq!(&src[prelude.inputs[0].expr.node.span.clone()], "$x :number");
        assert_eq!(prelude.locals.len(), 1);
        assert_eq!(prelude.locals[0].name, "y");
        assert_eq!(&src[prelude.locals[0].expr.node.span.clone()], "$x");
    }

    #[test]
    fn rejects_local_missing_name() {
        let src = ".local $ = {1}";
        let err = parse_declaration_prelude(src, line_ctx(1)).expect_err("must fail");
        assert!(matches!(
            err,
            crate::compile::CompileError::InvalidExpr { .. }
        ));
    }

    #[test]
    fn malformed_input_declaration_reports_expected_and_found() {
        let src = ".input oops";
        let err = parse_declaration_prelude(src, line_ctx(1)).expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidExpr {
                expected,
                found,
                line,
                column,
            } => {
                assert_eq!(line, 1);
                assert_eq!(column, 1);
                assert_eq!(expected, Some(".input {<expression>}"));
                assert!(found.as_deref().is_some_and(|f| f.contains(".input")));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn malformed_local_declaration_reports_expected_and_found() {
        let src = ".local $name = nope";
        let err = parse_declaration_prelude(src, line_ctx(1)).expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidExpr {
                expected,
                found,
                line,
                column,
            } => {
                assert_eq!(line, 1);
                assert_eq!(column, 1);
                assert_eq!(expected, Some(".local $name = {<expression>}"));
                assert!(found.as_deref().is_some_and(|f| f.contains(".local")));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parses_match_declaration_prelude() {
        let src = ".match $n one {{One}} * {{Other}}";
        let payload = parse_match_declaration_prelude(src, line_ctx(1))
            .expect("parsed")
            .expect("present");
        assert_eq!(payload.selectors.len(), 1);
        assert_eq!(&src[payload.selectors[0].span.clone()], "n");
        assert_eq!(payload.selectors[0].canonical, "n");
        assert_eq!(payload.variants.len(), 2);
        assert_eq!(&src[payload.variants[0].pattern.span.clone()], "One");
        assert_eq!(&src[payload.variants[1].pattern.span.clone()], "Other");
    }

    #[test]
    fn match_prelude_decodes_variant_keys() {
        let src = ".match $n |a\\|b| {{Hit}} * {{Other}}";
        let payload = parse_match_declaration_prelude(src, line_ctx(1))
            .expect("parsed")
            .expect("present");
        assert_eq!(payload.variants.len(), 2);
        assert_eq!(payload.variants[0].keys[0].key, "a|b");
        assert_eq!(payload.variants[1].keys[0].key, "other");
        assert_eq!(payload.duplicate_keys.len(), 1);
        assert_eq!(payload.duplicate_keys[0].key, "a|b");
    }

    #[test]
    fn malformed_match_head_reports_whitespace_diagnostic() {
        let src = ".match\u{16A0} $x * {{ok}}";
        let err = parse_match_declaration_prelude(src, line_ctx(1)).expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidExpr {
                line,
                column,
                expected,
                found,
            } => {
                assert_eq!(line, 1);
                assert_eq!(column, 7);
                assert_eq!(expected, Some("whitespace after '.match'"));
                assert!(found.as_deref().is_some_and(|f| f.contains('ᚠ')));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_normalized_variant_keys() {
        let keys = vec![
            VariantKeyOccurrence {
                key: "Ḍ̇".to_string(),
                is_default: false,
                line: 1,
                column: 7,
                display: "Ḍ̇".to_string(),
            },
            VariantKeyOccurrence {
                key: "Ḍ̇".to_string(),
                is_default: false,
                line: 1,
                column: 15,
                display: "Ḍ̇".to_string(),
            },
        ];
        let err = validate_no_duplicate_single_selector_keys(&keys).expect_err("must fail");
        assert!(matches!(
            err,
            crate::compile::CompileError::DuplicateVariant { .. }
        ));
    }

    #[test]
    fn parse_prefixed_variable_token_rejects_invalid_identifier() {
        let err = parse_prefixed_variable_token("$-bad", 1, 1).expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidVar {
                line,
                column,
                expected,
                found,
            } => {
                assert_eq!(line, 1);
                assert_eq!(column, 2);
                assert_eq!(expected, Some("valid identifier after '$'"));
                assert!(
                    found
                        .as_deref()
                        .is_some_and(|value| value.contains("$-bad"))
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_function_spec_node_reports_missing_equals_for_known_function() {
        let source = "number minimumFractionDigits";
        let option_spans = core::iter::once(7..source.len()).collect();
        let function = crate::syntax::ast::FunctionSpecNode {
            name: "number",
            name_span: 0..6,
            options: vec!["minimumFractionDigits"],
            option_spans,
            structured_options: Vec::new(),
            attributes: Vec::new(),
        };
        let err = parse_function_spec_node(source, &function, line_ctx(1)).expect_err("must fail");
        match err {
            crate::compile::CompileError::InvalidExpr {
                expected, found, ..
            } => {
                assert_eq!(
                    expected,
                    Some("option token 'key=value' or attribute '@name'")
                );
                assert!(
                    found
                        .as_deref()
                        .is_some_and(|value| value.contains("minimumFractionDigits"))
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
