// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{borrow::ToOwned, string::String};

use super::*;

use super::local_eval::{
    apply_literal_function, local_function_may_fail_select, normalize_local_function_expression,
};
use super::lower_expression_node_to_part;
use super::pattern::FunctionOriginContext;
use crate::common::text::parse_number_literal;

pub(super) struct DeclarationBindings {
    pub(super) locals: BTreeMap<String, LocalValue>,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) input_aliases: BTreeMap<String, String>,
    pub(super) local_functions: BTreeMap<String, DeclFunction>,
    pub(super) input_functions: BTreeMap<String, DeclFunction>,
    pub(super) slots: BTreeMap<String, u32>,
}

struct InputDeclarationBindings {
    aliases: BTreeMap<String, String>,
    functions: BTreeMap<String, DeclFunction>,
}

struct LocalDeclarationAnalysis {
    values: BTreeMap<String, LocalValue>,
    aliases: BTreeMap<String, String>,
    functions: BTreeMap<String, DeclFunction>,
}

#[derive(Clone)]
pub(super) enum LocalValue {
    Literal {
        value: String,
        kind: OperandLiteralKind,
    },
    Call,
    UnknownFunction,
}

impl LocalValue {
    pub(super) fn as_literal(&self) -> Option<&str> {
        match self {
            Self::Literal { value, .. } => Some(value.as_str()),
            Self::Call | Self::UnknownFunction => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct DeclFunction {
    pub(super) operand: Operand,
    pub(super) func: FunctionSpec,
}

impl DeclFunction {
    pub(super) fn from_part(part: Part) -> Option<Self> {
        match part {
            Part::Call(CallExpr { operand, func, .. }) => Some(Self { operand, func }),
            _ => None,
        }
    }

    pub(super) fn into_part(self, fallback: Option<String>) -> Part {
        Part::Call(CallExpr {
            operand: self.operand,
            func: self.func,
            fallback,
        })
    }
}

pub(super) fn collect_declaration_bindings(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<DeclarationBindings, CompileError> {
    let input_bindings = collect_input_declarations(source, declarations, ctx, function_origin);
    let local_analysis = analyze_local_declarations(
        source,
        declarations,
        ctx,
        function_origin,
        &input_bindings.functions,
    )?;
    let mut ordered = declarations
        .inputs
        .iter()
        .filter(|decl| bindings_input_needs_slot(&input_bindings.functions, &decl.canonical))
        .map(|decl| (decl.expr.node.span.start, decl.canonical.as_str()))
        .chain(
            declarations
                .locals
                .iter()
                .filter(|decl| {
                    local_analysis
                        .values
                        .get(&decl.canonical)
                        .is_none_or(|value| value.as_literal().is_none())
                })
                .map(|decl| (decl.expr.node.span.start, decl.canonical.as_str())),
        )
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(start, _)| *start);
    let slots = ordered
        .into_iter()
        .enumerate()
        .map(|(slot, (_, name))| {
            u32::try_from(slot)
                .map(|slot| (name.to_owned(), slot))
                .map_err(|_| CompileError::size_overflow("declaration slots"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    Ok(DeclarationBindings {
        locals: local_analysis.values,
        aliases: local_analysis.aliases,
        input_aliases: input_bindings.aliases,
        local_functions: local_analysis.functions,
        input_functions: input_bindings.functions,
        slots,
    })
}

fn bindings_input_needs_slot(functions: &BTreeMap<String, DeclFunction>, name: &str) -> bool {
    functions.contains_key(name)
}

fn collect_input_declarations(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> InputDeclarationBindings {
    let mut functions = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for declaration in &declarations.inputs {
        if let Ok(parsed) =
            lower_expression_node_to_part(source, &declaration.expr.node, ctx, function_origin)
            && let Some((name, function)) = extract_declared_function_part(parsed)
        {
            functions.insert(name, function);
        }

        aliases.insert(
            declaration.canonical.clone(),
            declaration.source_name.clone(),
        );
    }

    InputDeclarationBindings { aliases, functions }
}

fn analyze_local_declarations(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
    input_functions: &BTreeMap<String, DeclFunction>,
) -> Result<LocalDeclarationAnalysis, CompileError> {
    let mut values = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    let mut functions = BTreeMap::new();

    for declaration in &declarations.locals {
        let name = declaration.canonical.clone();
        let parsed =
            lower_expression_node_to_part(source, &declaration.expr.node, ctx, function_origin)?;
        let direct_literal_kind = local_literal_kind(source, &declaration.expr.node);

        if let Part::Var(alias) = &parsed {
            aliases.insert(name.clone(), alias.clone());
            // A local alias still denotes the already-analyzed value. Keep a
            // value binding as well so later re-annotations preserve the
            // structured numeric call instead of falling back to a runtime
            // variable lookup for the alias name.
            if let Some(value) = values.get(&canonicalize_identifier(alias)).cloned() {
                values.insert(name.clone(), value);
            }
        }

        if let Some(function) = normalize_declared_function_part(parsed.clone(), input_functions) {
            functions.insert(name.clone(), function);
        }

        if let Some(value) = evaluate_local_value(parsed, direct_literal_kind, &values) {
            values.insert(name, value);
        }
    }

    Ok(LocalDeclarationAnalysis {
        values,
        aliases,
        functions,
    })
}

fn extract_declared_function_part(part: Part) -> Option<(String, DeclFunction)> {
    match part {
        Part::Call(CallExpr {
            operand: Operand::Var(var),
            func,
            ..
        }) => Some((
            var.clone(),
            DeclFunction {
                operand: Operand::Var(var),
                func,
            },
        )),
        _ => None,
    }
}

fn normalize_declared_function_part(
    part: Part,
    input_functions: &BTreeMap<String, DeclFunction>,
) -> Option<DeclFunction> {
    DeclFunction::from_part(part)
        .map(|function| normalize_local_function_expression(function, input_functions))
}

fn evaluate_local_value(
    part: Part,
    direct_literal_kind: Option<OperandLiteralKind>,
    known_values: &BTreeMap<String, LocalValue>,
) -> Option<LocalValue> {
    match part {
        Part::Literal(value) => Some(LocalValue::Literal {
            value,
            kind: direct_literal_kind.unwrap_or(OperandLiteralKind::String),
        }),
        Part::Call(CallExpr { operand, func, .. }) => {
            if local_function_may_fail_select(&func) {
                return None;
            }
            let base = match operand {
                Operand::Literal { value, kind } => Some(Operand::Literal { value, kind }),
                Operand::Var(var) => match known_values.get(&var) {
                    Some(LocalValue::Literal { value, kind }) => Some(Operand::Literal {
                        value: value.clone(),
                        kind: *kind,
                    }),
                    Some(LocalValue::Call) | Some(LocalValue::UnknownFunction) | None => None,
                },
                Operand::Local(_) | Operand::Call(_) => None,
            };
            // Numeric calls and calls over a non-literal declaration remain
            // runtime values; folding them would lose exact payload/options.
            if matches!(
                func.name.as_str(),
                "string" | "number" | "integer" | "offset"
            ) || base.is_none()
            {
                Some(LocalValue::Call)
            } else {
                let Some(Operand::Literal { value, .. }) = base else {
                    return None;
                };
                Some(apply_literal_function(value, &func))
            }
        }
        _ => None,
    }
}

fn local_literal_kind(
    source: &str,
    expr: &crate::compiler::syntax::ast::ExpressionNode<'_>,
) -> Option<OperandLiteralKind> {
    let literal = match &expr.payload {
        Some(crate::compiler::syntax::ast::ExpressionPayloadNode::Literal(literal))
            if !literal.is_markup && literal.function.is_none() =>
        {
            literal
        }
        _ => return None,
    };

    let raw = source
        .get(literal.value_span.clone())
        .unwrap_or(literal.value.trim());
    Some(if raw.starts_with('|') {
        OperandLiteralKind::String
    } else if parse_number_literal(raw).is_some() {
        OperandLiteralKind::Number
    } else {
        OperandLiteralKind::String
    })
}
