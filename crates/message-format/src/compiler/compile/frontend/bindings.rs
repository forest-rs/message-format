// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{borrow::ToOwned, string::String};

use super::*;

use super::lower_expression_node_to_part;
use super::pattern::FunctionOriginContext;
use crate::common::text::parse_number_literal;

pub(super) struct DeclarationBindings {
    pub(super) literals: BTreeMap<String, LocalLiteral>,
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
    literals: BTreeMap<String, LocalLiteral>,
    aliases: BTreeMap<String, String>,
    functions: BTreeMap<String, DeclFunction>,
}

#[derive(Clone)]
pub(super) struct LocalLiteral {
    pub(super) value: String,
    pub(super) kind: OperandLiteralKind,
}

impl LocalLiteral {
    pub(super) fn as_str(&self) -> &str {
        self.value.as_str()
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
                .filter(|decl| !local_analysis.literals.contains_key(&decl.canonical))
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
        literals: local_analysis.literals,
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
    let mut literals = BTreeMap::new();
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
            if let Some(value) = literals.get(&canonicalize_identifier(alias)).cloned() {
                literals.insert(name.clone(), value);
            }
        }

        if let Some(function) = normalize_declared_function_part(parsed.clone(), input_functions) {
            functions.insert(name.clone(), function);
        }

        if let Some(value) = evaluate_local_literal(parsed, direct_literal_kind) {
            literals.insert(name, value);
        }
    }

    Ok(LocalDeclarationAnalysis {
        literals,
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

fn normalize_local_function_expression(
    expression: DeclFunction,
    input_functions: &BTreeMap<String, DeclFunction>,
) -> DeclFunction {
    let Operand::Var(operand) = &expression.operand else {
        return expression;
    };
    let Some(input_expression) = input_functions.get(operand) else {
        return expression;
    };
    DeclFunction {
        operand: Operand::Call(Box::new(CallExpr {
            operand: input_expression.operand.clone(),
            func: input_expression.func.clone(),
            fallback: None,
        })),
        func: expression.func,
    }
}

pub(super) fn resolve_alias(
    name: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<String, CompileError> {
    let mut name = name.to_owned();
    for _ in 0..8 {
        let Some(next) = aliases.get(&name).cloned() else {
            return Ok(name);
        };
        name = next;
    }
    Err(CompileError::alias_resolution_overflow(name))
}

fn evaluate_local_literal(
    part: Part,
    direct_literal_kind: Option<OperandLiteralKind>,
) -> Option<LocalLiteral> {
    match part {
        Part::Literal(value) => Some(LocalLiteral {
            value,
            kind: direct_literal_kind.unwrap_or(OperandLiteralKind::String),
        }),
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
