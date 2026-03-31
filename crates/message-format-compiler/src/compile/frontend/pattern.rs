// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::syntax::{
    ident::canonicalize_identifier, literal::parse_number_literal, span::quoted_snippet,
};

use super::*;

#[derive(Clone, Copy)]
pub(in crate::compile) struct FunctionOriginContext {
    pub(in crate::compile) source_id: SourceId,
    pub(in crate::compile) base_byte: usize,
}

pub(super) fn lower_pattern_node_to_parts(
    source: &str,
    pattern: &crate::syntax::ast::PatternNode<'_>,
    ctx: SourceContext,
    options: CompileOptions,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Vec<Part>, CompileError> {
    if pattern.span.start > pattern.span.end || pattern.span.end > source.len() {
        let (line, _) = ctx.location(source, 0);
        return Err(CompileError::invalid_expr(line));
    }
    let pattern_source = &source[pattern.span.clone()];
    if pattern_source.trim() == "." {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::invalid_expr_detail(
            line,
            col,
            "pattern content",
            "\".\"",
        ));
    }
    let (line, _) = ctx.location(source, 0);
    validate_balanced_braces(pattern_source, line)?;

    let mut parts = Vec::new();
    for segment in &pattern.segments {
        match segment {
            crate::syntax::ast::PatternSegmentNode::Text { value, .. } => {
                let text = decode_text_fragment(value, line)?;
                if !text.is_empty() {
                    parts.push(Part::Text(text));
                }
            }
            crate::syntax::ast::PatternSegmentNode::Expression(expr) => {
                let part = lower_expression_node_to_part_with_context(
                    source,
                    expr.as_ref(),
                    ExpressionLoweringContext {
                        default_bidi_isolation: options.default_bidi_isolation,
                        allow_default_bidi_rewrite: true,
                        ctx,
                        function_origin,
                    },
                )?;
                // Self-closing markup: emit open + close in sequence.
                if is_self_close_markup(expr) {
                    if let Part::MarkupOpen { ref name, .. } = part {
                        let close_name = name.clone();
                        parts.push(part);
                        parts.push(Part::MarkupClose {
                            name: close_name,
                            options: Vec::new(),
                        });
                        continue;
                    }
                }
                parts.push(part);
            }
        }
    }

    Ok(parts)
}

pub(super) fn lower_expression_node_to_part(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Part, CompileError> {
    lower_expression_node_to_part_with_context(
        source,
        expr,
        ExpressionLoweringContext::without_default_bidi_rewrite(ctx, function_origin),
    )
}

#[derive(Clone, Copy)]
struct ExpressionLoweringContext {
    default_bidi_isolation: bool,
    allow_default_bidi_rewrite: bool,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
}

impl ExpressionLoweringContext {
    const fn without_default_bidi_rewrite(
        ctx: SourceContext,
        function_origin: Option<FunctionOriginContext>,
    ) -> Self {
        Self {
            default_bidi_isolation: false,
            allow_default_bidi_rewrite: false,
            ctx,
            function_origin,
        }
    }
}

fn lower_expression_node_to_part_with_context(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    context: ExpressionLoweringContext,
) -> Result<Part, CompileError> {
    let Some(payload) = &expr.payload else {
        return infer_non_select_payload_error(source, expr, context.ctx);
    };
    lower_expression_payload_node_to_part(source, payload, context)
}

fn infer_non_select_payload_error(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
) -> Result<Part, CompileError> {
    let (line, column) = ctx.location(source, expr.span.start);
    if let Some(hint) = expr.diag_hint {
        return match hint {
            crate::syntax::ast::ExpressionDiagnosticHint::FunctionNameRequiredAfterColon => {
                Err(CompileError::invalid_expr_detail(
                    line,
                    column,
                    "function name after ':'",
                    "end of expression",
                ))
            }
            _ => Err(CompileError::invalid_expr_detail(
                line,
                column,
                "typed expression payload",
                "missing expression payload",
            )),
        };
    }
    Err(CompileError::invalid_expr_detail(
        line,
        column,
        "typed expression payload",
        "missing expression payload",
    ))
}

fn lower_expression_payload_node_to_part(
    source: &str,
    payload: &crate::syntax::ast::ExpressionPayloadNode<'_>,
    context: ExpressionLoweringContext,
) -> Result<Part, CompileError> {
    let ctx = context.ctx;
    let (line, _) = ctx.location(source, 0);
    match payload {
        crate::syntax::ast::ExpressionPayloadNode::Var(var) => {
            let mut full = String::from("$");
            full.push_str(var.name);
            let dollar_byte = var.name_span.start.saturating_sub(1);
            let (_, column) = ctx.location(source, dollar_byte);
            let var = crate::syntax::semantic::parse_prefixed_variable_token(&full, line, column)?;
            if context.default_bidi_isolation && context.allow_default_bidi_rewrite {
                return Ok(Part::Call(CallExpr {
                    operand: Operand::Var(var),
                    func: FunctionSpec {
                        name: String::from("string"),
                        options: Vec::new(),
                        origin: None,
                    },
                    fallback: None,
                }));
            }
            Ok(Part::Var(var))
        }
        crate::syntax::ast::ExpressionPayloadNode::Call(call) => {
            let mut func_spec =
                crate::syntax::semantic::parse_function_spec_node(source, &call.function, ctx)?;
            func_spec.origin = function_spec_origin(&call.function, source, context);
            match &call.operand {
                crate::syntax::ast::CallOperandNode::Var(var) => {
                    let mut full = String::from("$");
                    full.push_str(var.name);
                    let dollar_byte = var.name_span.start.saturating_sub(1);
                    let (_, column) = ctx.location(source, dollar_byte);
                    let var = crate::syntax::semantic::parse_prefixed_variable_token(
                        &full, line, column,
                    )?;
                    Ok(Part::Call(CallExpr {
                        operand: Operand::Var(var),
                        func: func_spec,
                        fallback: None,
                    }))
                }
                crate::syntax::ast::CallOperandNode::Literal { .. } => Ok(Part::Call(CallExpr {
                    operand: lower_operand_literal(source, &call.operand, line)?,
                    func: func_spec,
                    fallback: None,
                })),
            }
        }
        crate::syntax::ast::ExpressionPayloadNode::Markup(markup) => {
            let options = lower_markup_options(source, &markup.options, ctx)?;
            match markup.kind {
                crate::syntax::ast::MarkupKind::Open => Ok(Part::MarkupOpen {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
                crate::syntax::ast::MarkupKind::Close => Ok(Part::MarkupClose {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
                crate::syntax::ast::MarkupKind::SelfClose => Ok(Part::MarkupOpen {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
            }
        }
        crate::syntax::ast::ExpressionPayloadNode::Literal(literal) => {
            if literal.is_markup {
                return Ok(Part::Literal(String::new()));
            }
            if let Some(func) = &literal.function {
                let mut func_spec =
                    crate::syntax::semantic::parse_function_spec_node(source, func, ctx)?;
                func_spec.origin = function_spec_origin(func, source, context);
                let value = if literal.value.trim().is_empty() {
                    String::new()
                } else {
                    parse_literal_text(literal.value.trim(), line)?
                };
                return Ok(Part::Call(CallExpr {
                    operand: lower_literal_expression_operand(&literal.value_span, value, source),
                    func: func_spec,
                    fallback: None,
                }));
            }
            let value = parse_literal_text(literal.value.trim(), line)?;
            if context.default_bidi_isolation && context.allow_default_bidi_rewrite {
                return Ok(Part::Call(CallExpr {
                    operand: lower_literal_expression_operand(&literal.value_span, value, source),
                    func: FunctionSpec {
                        name: String::from("string"),
                        options: Vec::new(),
                        origin: None,
                    },
                    fallback: None,
                }));
            }
            Ok(Part::Literal(value))
        }
    }
}

fn lower_operand_literal(
    source: &str,
    operand: &crate::syntax::ast::CallOperandNode<'_>,
    line: usize,
) -> Result<Operand, CompileError> {
    match operand {
        crate::syntax::ast::CallOperandNode::Var(_) => unreachable!("caller only passes literals"),
        crate::syntax::ast::CallOperandNode::Literal { value, value_span } => Ok(
            lower_literal_expression_operand(value_span, parse_literal_text(value, line)?, source),
        ),
    }
}

fn lower_literal_expression_operand(
    value_span: &core::ops::Range<usize>,
    value: String,
    source: &str,
) -> Operand {
    match classify_operand_literal_kind(source, value_span, &value) {
        OperandLiteralKind::String => Operand::literal(value),
        OperandLiteralKind::Number => Operand::number_literal(value),
    }
}

fn classify_operand_literal_kind(
    source: &str,
    value_span: &core::ops::Range<usize>,
    value: &str,
) -> OperandLiteralKind {
    let raw = source.get(value_span.clone()).unwrap_or(value);
    if raw.starts_with('|') {
        OperandLiteralKind::String
    } else if parse_number_literal(raw).is_some() {
        OperandLiteralKind::Number
    } else {
        OperandLiteralKind::String
    }
}

fn is_self_close_markup(expr: &crate::syntax::ast::ExpressionNode<'_>) -> bool {
    matches!(
        &expr.payload,
        Some(crate::syntax::ast::ExpressionPayloadNode::Markup(m))
            if m.kind == crate::syntax::ast::MarkupKind::SelfClose
    )
}

fn lower_markup_options(
    source: &str,
    options: &[crate::syntax::ast::OptionNode<'_>],
    ctx: SourceContext,
) -> Result<Vec<FunctionOption>, CompileError> {
    let mut lowered = Vec::with_capacity(options.len());
    let mut seen_keys = BTreeSet::new();
    for opt in options {
        let (line, column) = ctx.location(source, opt.span.start);
        if !seen_keys.insert(opt.name) {
            return Err(CompileError::duplicate_option_name_detail(
                line,
                column,
                "unique option names",
                format!("duplicate option {}", quoted_snippet(opt.name)),
            ));
        }
        lowered.push(FunctionOption {
            key: opt.name.to_string(),
            value: match &opt.value {
                crate::syntax::ast::OptionValue::Literal(lit) => {
                    FunctionOptionValue::Literal(parse_literal_text(lit, line)?)
                }
                crate::syntax::ast::OptionValue::Variable(var) => {
                    FunctionOptionValue::Var(canonicalize_identifier(var))
                }
            },
        });
    }
    Ok(lowered)
}

fn function_spec_origin(
    function: &crate::syntax::ast::FunctionSpecNode<'_>,
    source: &str,
    context: ExpressionLoweringContext,
) -> Option<SourceSpan> {
    let function_origin = context.function_origin?;
    let start = function.name_span.start.saturating_sub(1);
    let end = function
        .option_spans
        .last()
        .map_or(function.name_span.end, |span| span.end);
    let absolute_start = function_origin.base_byte + start;
    let absolute_end = function_origin.base_byte + end;
    let (line, column) = context.ctx.location(source, start);
    Some(SourceSpan {
        source_id: function_origin.source_id,
        byte_start: absolute_start,
        byte_end: absolute_end,
        line: Some(line),
        column: Some(column),
    })
}
