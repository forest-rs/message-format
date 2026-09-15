// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, string::String, string::ToString, vec::Vec};
use core::ops::Range;

use crate::common::text::parse_number_literal;
use crate::compiler::syntax::{ident::canonicalize_identifier, span::quoted_snippet};

use super::bindings::{
    DeclarationPlan, PlannedValue, ResolvedReference, ResolvedValue, ordered_declarations,
};
use super::*;

#[derive(Clone, Copy)]
pub(in crate::compiler::compile) struct FunctionOriginContext {
    pub(in crate::compiler::compile) source_id: SourceId,
    pub(in crate::compiler::compile) base_byte: usize,
}

pub(super) fn lower_pattern_node_to_parts(
    source: &str,
    pattern: &crate::compiler::syntax::ast::PatternNode<'_>,
    ctx: SourceContext,
    options: CompileOptions,
    function_origin: Option<FunctionOriginContext>,
    plan: Option<&DeclarationPlan>,
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
            crate::compiler::syntax::ast::PatternSegmentNode::Text { value, .. } => {
                let text = decode_text_fragment(value, line)?;
                if !text.is_empty() {
                    parts.push(Part::Text(text));
                }
            }
            crate::compiler::syntax::ast::PatternSegmentNode::Expression(expr) => {
                let part = lower_expression_node_to_part_with_context(
                    source,
                    expr.as_ref(),
                    ExpressionLoweringContext {
                        default_bidi_isolation: options.default_bidi_isolation,
                        ctx,
                        function_origin,
                    },
                    plan,
                    None,
                )?;
                // Self-closing markup: emit open + close in sequence.
                if is_self_close_markup(expr)
                    && let Part::MarkupOpen { ref name, .. } = part
                {
                    let close_name = name.clone();
                    parts.push(part);
                    parts.push(Part::MarkupClose {
                        name: close_name,
                        options: Vec::new(),
                    });
                    continue;
                }
                parts.push(part);
            }
        }
    }

    Ok(parts)
}

pub(super) fn lower_expression_node_to_part(
    source: &str,
    expr: &crate::compiler::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Part, CompileError> {
    lower_expression_node_to_part_with_context(
        source,
        expr,
        ExpressionLoweringContext::without_default_bidi(ctx, function_origin),
        None,
        None,
    )
}

pub(super) fn lower_declaration_prelude(
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    plan: &DeclarationPlan,
) -> Result<Vec<Part>, CompileError> {
    let ordered = ordered_declarations(declarations);
    let mut parts = Vec::with_capacity(ordered.len());
    for (_, name, _) in ordered {
        let Some((slot, value)) = plan.declaration(name) else {
            continue;
        };
        let fallback = format!("{{${name}}}");
        let value = materialize_planned_value(value, name, plan, &fallback)?;
        parts.push(Part::Bind {
            slot,
            fallback,
            value: Box::new(value),
        });
    }
    Ok(parts)
}

#[derive(Clone, Copy)]
struct ExpressionLoweringContext {
    default_bidi_isolation: bool,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
}

impl ExpressionLoweringContext {
    const fn without_default_bidi(
        ctx: SourceContext,
        function_origin: Option<FunctionOriginContext>,
    ) -> Self {
        Self {
            default_bidi_isolation: false,
            ctx,
            function_origin,
        }
    }
}

fn lower_expression_node_to_part_with_context(
    source: &str,
    expr: &crate::compiler::syntax::ast::ExpressionNode<'_>,
    context: ExpressionLoweringContext,
    plan: Option<&DeclarationPlan>,
    excluded: Option<&str>,
) -> Result<Part, CompileError> {
    let Some(payload) = &expr.payload else {
        return infer_non_select_payload_error(source, expr, context.ctx);
    };
    lower_expression_payload_node_to_part(source, payload, context, plan, excluded)
}

fn infer_non_select_payload_error(
    source: &str,
    expr: &crate::compiler::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
) -> Result<Part, CompileError> {
    let (line, column) = ctx.location(source, expr.span.start);
    if let Some(hint) = expr.diag_hint {
        return match hint {
            crate::compiler::syntax::ast::ExpressionDiagnosticHint::FunctionNameRequiredAfterColon => {
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
    payload: &crate::compiler::syntax::ast::ExpressionPayloadNode<'_>,
    context: ExpressionLoweringContext,
    plan: Option<&DeclarationPlan>,
    excluded: Option<&str>,
) -> Result<Part, CompileError> {
    let ctx = context.ctx;
    let (line, _) = ctx.location(source, 0);
    match payload {
        crate::compiler::syntax::ast::ExpressionPayloadNode::Var(var) => {
            let mut full = String::from("$");
            full.push_str(var.name);
            let dollar_byte = var.name_span.start.saturating_sub(1);
            let (_, column) = ctx.location(source, dollar_byte);
            let var = crate::compiler::syntax::semantic::parse_prefixed_variable_token(
                &full, line, column,
            )?;
            if context.default_bidi_isolation {
                let mut call = CallExpr {
                    operand: Operand::Var(var),
                    func: FunctionSpec::new("string").option_literal("u:dir", "auto"),
                    fallback: None,
                };
                resolve_call(&mut call, plan, excluded);
                return Ok(Part::Call(call));
            }
            if let Some(plan) = plan {
                return Ok(part_from_reference(
                    plan.resolve_reference(&var, excluded),
                    var,
                    plan,
                ));
            }
            Ok(Part::Var(var))
        }
        crate::compiler::syntax::ast::ExpressionPayloadNode::Call(call) => {
            let mut func_spec = crate::compiler::syntax::semantic::parse_function_spec_node(
                source,
                &call.function,
                ctx,
            )?;
            func_spec.origin = function_spec_origin(&call.function, source, context);
            apply_default_bidi_direction(&mut func_spec, context);
            match &call.operand {
                crate::compiler::syntax::ast::CallOperandNode::Var(var) => {
                    let mut full = String::from("$");
                    full.push_str(var.name);
                    let dollar_byte = var.name_span.start.saturating_sub(1);
                    let (_, column) = ctx.location(source, dollar_byte);
                    let var = crate::compiler::syntax::semantic::parse_prefixed_variable_token(
                        &full, line, column,
                    )?;
                    let mut call = CallExpr {
                        operand: Operand::Var(var),
                        func: func_spec,
                        fallback: None,
                    };
                    resolve_call(&mut call, plan, excluded);
                    Ok(Part::Call(call))
                }
                crate::compiler::syntax::ast::CallOperandNode::Literal { .. } => {
                    let mut call = CallExpr {
                        operand: lower_operand_literal(source, &call.operand, line)?,
                        func: func_spec,
                        fallback: None,
                    };
                    if let Some(plan) = plan {
                        resolve_call_options(&mut call, plan);
                    }
                    Ok(Part::Call(call))
                }
            }
        }
        crate::compiler::syntax::ast::ExpressionPayloadNode::Markup(markup) => {
            let options = lower_markup_options(source, &markup.options, ctx)?;
            match markup.kind {
                crate::compiler::syntax::ast::MarkupKind::Open => Ok(Part::MarkupOpen {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
                crate::compiler::syntax::ast::MarkupKind::Close => Ok(Part::MarkupClose {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
                crate::compiler::syntax::ast::MarkupKind::SelfClose => Ok(Part::MarkupOpen {
                    name: canonicalize_identifier(markup.identifier),
                    options,
                }),
            }
        }
        crate::compiler::syntax::ast::ExpressionPayloadNode::Literal(literal) => {
            if literal.is_markup {
                return Ok(Part::Literal(String::new()));
            }
            if let Some(func) = &literal.function {
                let mut func_spec =
                    crate::compiler::syntax::semantic::parse_function_spec_node(source, func, ctx)?;
                func_spec.origin = function_spec_origin(func, source, context);
                apply_default_bidi_direction(&mut func_spec, context);
                let value = if literal.value.trim().is_empty() {
                    String::new()
                } else {
                    parse_literal_text(literal.value.trim(), line)?
                };
                let mut call = CallExpr {
                    operand: lower_literal_expression_operand(&literal.value_span, value, source),
                    func: func_spec,
                    fallback: None,
                };
                if let Some(plan) = plan {
                    resolve_call_options(&mut call, plan);
                }
                return Ok(Part::Call(call));
            }
            let value = parse_literal_text(literal.value.trim(), line)?;
            if context.default_bidi_isolation {
                return Ok(Part::Call(CallExpr {
                    operand: lower_literal_expression_operand(&literal.value_span, value, source),
                    func: FunctionSpec::new("string").option_literal("u:dir", "auto"),
                    fallback: None,
                }));
            }
            Ok(Part::Literal(value))
        }
    }
}

fn materialize_planned_value(
    value: &PlannedValue,
    name: &str,
    plan: &DeclarationPlan,
    fallback: &str,
) -> Result<Part, CompileError> {
    match value {
        PlannedValue::Function(function) => {
            let mut function = function.clone();
            function.fallback = Some(fallback.to_string());
            resolve_call(&mut function, Some(plan), Some(name));
            Ok(Part::Call(function))
        }
        PlannedValue::Alias { evaluation, .. } => Ok(part_from_reference(
            plan.resolve_reference(evaluation, Some(name)),
            evaluation.clone(),
            plan,
        )),
        PlannedValue::Argument(_) | PlannedValue::Literal(_) | PlannedValue::Unresolved => {
            Err(CompileError::internal("unstored planned declaration"))
        }
    }
}

fn resolve_call(call: &mut CallExpr, plan: Option<&DeclarationPlan>, excluded: Option<&str>) {
    if let Some(plan) = plan
        && let Operand::Var(var) = &call.operand
    {
        let var = var.clone();
        let reference = plan.resolve_reference(&var, excluded);
        if call.fallback.is_none()
            && let Some(name) = reference.fallback.as_deref()
        {
            call.fallback = Some(format!("{{${name}}}"));
        }
        call.operand = operand_from_reference(reference, var, plan);
    }
    if let Some(plan) = plan {
        resolve_call_options(call, plan);
    }
}

fn part_from_reference(
    reference: ResolvedReference,
    original: String,
    plan: &DeclarationPlan,
) -> Part {
    match reference.value {
        ResolvedValue::Unchanged => Part::Var(original),
        ResolvedValue::Local(slot) => Part::Local(slot),
        ResolvedValue::Literal(literal) => Part::Literal(literal.value),
        ResolvedValue::Function(mut function) => {
            resolve_call(&mut function, Some(plan), reference.fallback.as_deref());
            function.fallback = reference.fallback.map(|name| format!("{{${name}}}"));
            Part::Call(function)
        }
        ResolvedValue::Argument(name) => Part::Var(name),
    }
}

fn operand_from_reference(
    reference: ResolvedReference,
    original: String,
    plan: &DeclarationPlan,
) -> Operand {
    match reference.value {
        ResolvedValue::Unchanged => Operand::Var(original),
        ResolvedValue::Local(slot) => Operand::Local(slot),
        ResolvedValue::Literal(literal) => Operand::Literal {
            value: literal.value,
            kind: literal.kind,
        },
        ResolvedValue::Function(mut function) => {
            resolve_call(&mut function, Some(plan), reference.fallback.as_deref());
            function.fallback = None;
            Operand::Call(Box::new(function))
        }
        ResolvedValue::Argument(name) => Operand::Var(name),
    }
}

pub(super) fn resolve_call_options(call: &mut CallExpr, plan: &DeclarationPlan) {
    for option in &mut call.func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = plan.literal(var).map(|literal| literal.value.as_str()) else {
            if let Some(slot) = plan.slot(var) {
                option.value = FunctionOptionValue::LocalVar {
                    name: var.clone(),
                    slot,
                };
            }
            continue;
        };
        option.value = FunctionOptionValue::ResolvedVar {
            name: var.clone(),
            value: local_value.to_string(),
        };
    }
    if let Operand::Call(nested) = &mut call.operand {
        resolve_call_options(nested, plan);
    }
}

fn apply_default_bidi_direction(func: &mut FunctionSpec, context: ExpressionLoweringContext) {
    if context.default_bidi_isolation
        && func.name == "string"
        && !func.options.iter().any(|option| option.key == "u:dir")
    {
        func.options.push(FunctionOption::literal("u:dir", "auto"));
    }
}

fn lower_operand_literal(
    source: &str,
    operand: &crate::compiler::syntax::ast::CallOperandNode<'_>,
    line: usize,
) -> Result<Operand, CompileError> {
    match operand {
        crate::compiler::syntax::ast::CallOperandNode::Var(_) => {
            unreachable!("caller only passes literals")
        }
        crate::compiler::syntax::ast::CallOperandNode::Literal { value, value_span } => Ok(
            lower_literal_expression_operand(value_span, parse_literal_text(value, line)?, source),
        ),
    }
}

fn lower_literal_expression_operand(
    value_span: &Range<usize>,
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
    value_span: &Range<usize>,
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

fn is_self_close_markup(expr: &crate::compiler::syntax::ast::ExpressionNode<'_>) -> bool {
    matches!(
        &expr.payload,
        Some(crate::compiler::syntax::ast::ExpressionPayloadNode::Markup(m))
            if m.kind == crate::compiler::syntax::ast::MarkupKind::SelfClose
    )
}

fn lower_markup_options(
    source: &str,
    options: &[crate::compiler::syntax::ast::OptionNode<'_>],
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
                crate::compiler::syntax::ast::OptionValue::Literal(lit) => {
                    FunctionOptionValue::Literal(parse_literal_text(lit, line)?)
                }
                crate::compiler::syntax::ast::OptionValue::Variable(var) => {
                    FunctionOptionValue::Var(canonicalize_identifier(var))
                }
            },
        });
    }
    Ok(lowered)
}

fn function_spec_origin(
    function: &crate::compiler::syntax::ast::FunctionSpecNode<'_>,
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
