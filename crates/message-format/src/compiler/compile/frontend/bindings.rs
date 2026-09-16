// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::String;

use super::*;

use super::lower_expression_node_to_part;
use super::pattern::FunctionOriginContext;
use crate::common::text::parse_number_literal;
use crate::compiler::syntax::ast::{
    CallOperandNode, DeclarationPayloadNode, ExpressionNode, ExpressionPayloadNode, PatternNode,
    PatternSegmentNode, SyntaxDocument,
};
use crate::compiler::syntax::ident::strip_boundary_bidi_controls;
use crate::compiler::syntax::semantic::collect_var_refs;

pub(super) struct DeclarationPlan {
    entries: BTreeMap<String, PlannedDeclaration>,
}

struct PlannedDeclaration {
    value: PlannedValue,
    references: usize,
    slot: Option<u32>,
}

#[derive(Clone)]
pub(super) enum PlannedValue {
    Argument(String),
    Alias {
        evaluation: String,
        resolved: String,
    },
    Literal(LocalLiteral),
    Function(DeclFunction),
    Unresolved,
}

pub(super) struct ResolvedReference {
    pub(super) value: ResolvedValue,
    pub(super) fallback: Option<String>,
}

pub(super) enum ResolvedValue {
    Unchanged,
    Local(u32),
    Literal(LocalLiteral),
    Function(DeclFunction),
    Argument(String),
}

#[derive(Clone)]
pub(super) struct LocalLiteral {
    pub(super) value: String,
    pub(super) kind: OperandLiteralKind,
}

pub(super) type DeclFunction = CallExpr;

impl DeclarationPlan {
    pub(super) fn declaration(&self, name: &str) -> Option<(u32, &PlannedValue)> {
        let entry = self.entries.get(name)?;
        Some((entry.slot?, &entry.value))
    }

    pub(super) fn resolve_reference(
        &self,
        name: &str,
        excluded: Option<&str>,
    ) -> ResolvedReference {
        let canonical = canonicalize_identifier(name);
        if excluded == Some(canonical.as_str()) {
            return ResolvedReference {
                value: ResolvedValue::Unchanged,
                fallback: None,
            };
        }
        let resolved = self.resolved_name(&canonical);
        if excluded != Some(resolved.as_str())
            && let Some(slot) = self.slot(&canonical)
        {
            return ResolvedReference {
                value: ResolvedValue::Local(slot),
                fallback: Some(resolved),
            };
        }
        if let Some(literal) = self.literal(&canonical) {
            return ResolvedReference {
                value: ResolvedValue::Literal(literal.clone()),
                fallback: Some(resolved),
            };
        }
        if let Some(function) = self.function(&canonical).cloned()
            && self.slot(&canonical).is_none()
        {
            return ResolvedReference {
                value: ResolvedValue::Function(function),
                fallback: Some(resolved),
            };
        }
        ResolvedReference {
            value: ResolvedValue::Argument(self.argument_name(&resolved)),
            fallback: None,
        }
    }

    pub(super) fn slot(&self, name: &str) -> Option<u32> {
        self.resolved(name).and_then(|entry| entry.slot)
    }

    pub(super) fn literal(&self, name: &str) -> Option<&LocalLiteral> {
        match &self.resolved(name)?.value {
            PlannedValue::Literal(literal) => Some(literal),
            _ => None,
        }
    }

    pub(super) fn function(&self, name: &str) -> Option<&DeclFunction> {
        match &self.resolved(name)?.value {
            PlannedValue::Function(function) => Some(function),
            _ => None,
        }
    }

    pub(super) fn argument_name(&self, name: &str) -> String {
        match self.resolved(name).map(|entry| &entry.value) {
            Some(PlannedValue::Argument(source_name)) => source_name.clone(),
            _ => name.to_string(),
        }
    }

    pub(super) fn resolved_name(&self, name: &str) -> String {
        match self.entries.get(name) {
            Some(PlannedDeclaration {
                value: PlannedValue::Alias { resolved, .. },
                slot: None,
                ..
            }) => resolved.clone(),
            _ => name.to_string(),
        }
    }

    fn resolved(&self, name: &str) -> Option<&PlannedDeclaration> {
        let entry = self.entries.get(name)?;
        if entry.slot.is_none()
            && let PlannedValue::Alias {
                resolved: target, ..
            } = &entry.value
        {
            return self.entries.get(target);
        }
        Some(entry)
    }
}

pub(super) fn build_declaration_plan(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    doc: &SyntaxDocument<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<DeclarationPlan, CompileError> {
    let mut entries = BTreeMap::new();
    for declaration in &declarations.inputs {
        let parsed =
            lower_expression_node_to_part(source, &declaration.expr.node, ctx, function_origin);
        let value = parsed.ok().and_then(extract_input_function).map_or_else(
            || PlannedValue::Argument(declaration.source_name.clone()),
            PlannedValue::Function,
        );
        entries.insert(
            declaration.canonical.clone(),
            PlannedDeclaration {
                value,
                references: 0,
                slot: None,
            },
        );
    }

    for declaration in &declarations.locals {
        let parsed =
            lower_expression_node_to_part(source, &declaration.expr.node, ctx, function_origin)?;
        let value = plan_local_value(source, &declaration.expr.node, parsed, &entries);
        entries.insert(
            declaration.canonical.clone(),
            PlannedDeclaration {
                value,
                references: 0,
                slot: None,
            },
        );
    }

    for (name, references) in declaration_reference_counts(source, declarations, doc) {
        if let Some(entry) = entries.get_mut(&name) {
            entry.references = references;
        }
    }

    let ordered = ordered_declarations(declarations);
    let inlined = inlined_string_functions(&ordered, &entries);
    let mut next_slot = 0_u32;
    for (_, name, _) in &ordered {
        let stores = {
            let entry = entries
                .get(*name)
                .ok_or_else(|| CompileError::internal("missing planned declaration"))?;
            stores_runtime_value(entry, &entries, &inlined) && !inlined.contains(*name)
        };
        if stores {
            entries.get_mut(*name).expect("entry checked above").slot = Some(next_slot);
            next_slot = next_slot
                .checked_add(1)
                .ok_or_else(|| CompileError::size_overflow("declaration slots"))?;
        }
    }

    Ok(DeclarationPlan { entries })
}

fn plan_local_value(
    source: &str,
    expression: &ExpressionNode<'_>,
    parsed: Part,
    entries: &BTreeMap<String, PlannedDeclaration>,
) -> PlannedValue {
    if let Part::Var(alias) = &parsed {
        let canonical = canonicalize_identifier(alias);
        let resolved = match entries.get(&canonical).map(|entry| &entry.value) {
            Some(PlannedValue::Alias { resolved, .. }) => resolved.clone(),
            _ => canonical.clone(),
        };
        if let Some(PlannedValue::Literal(literal)) =
            entries.get(&resolved).map(|entry| &entry.value)
        {
            return PlannedValue::Literal(literal.clone());
        }
        return PlannedValue::Alias {
            evaluation: canonical,
            resolved,
        };
    }

    if let Part::Literal(value) = &parsed {
        return PlannedValue::Literal(LocalLiteral {
            value: value.clone(),
            kind: local_literal_kind(source, expression).unwrap_or(OperandLiteralKind::String),
        });
    }

    let Part::Call(function) = parsed else {
        return PlannedValue::Unresolved;
    };
    PlannedValue::Function(function)
}

fn extract_input_function(part: Part) -> Option<DeclFunction> {
    match part {
        Part::Call(CallExpr {
            operand: Operand::Var(var),
            func,
            ..
        }) => Some(CallExpr {
            operand: Operand::Var(var),
            func,
            fallback: None,
        }),
        _ => None,
    }
}

fn stores_runtime_value(
    entry: &PlannedDeclaration,
    entries: &BTreeMap<String, PlannedDeclaration>,
    inlined: &BTreeSet<String>,
) -> bool {
    match &entry.value {
        PlannedValue::Function(_) => true,
        PlannedValue::Alias { resolved, .. } => !matches!(
            entries.get(resolved),
            Some(PlannedDeclaration {
                value: PlannedValue::Function(_),
                ..
            }) if !inlined.contains(resolved)
        ),
        PlannedValue::Argument(_) | PlannedValue::Literal(_) | PlannedValue::Unresolved => false,
    }
}

pub(super) fn ordered_declarations<'a, 'source>(
    declarations: &'a crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'source>,
) -> Vec<(usize, &'a str, &'a ExpressionNode<'source>)> {
    let mut ordered = declarations
        .inputs
        .iter()
        .map(|decl| {
            (
                decl.expr.node.span.start,
                decl.canonical.as_str(),
                &decl.expr.node,
            )
        })
        .chain(declarations.locals.iter().map(|decl| {
            (
                decl.expr.node.span.start,
                decl.canonical.as_str(),
                &decl.expr.node,
            )
        }))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(start, _, _)| *start);
    ordered
}

fn inlined_string_functions(
    ordered: &[(usize, &str, &ExpressionNode<'_>)],
    entries: &BTreeMap<String, PlannedDeclaration>,
) -> BTreeSet<String> {
    ordered
        .windows(2)
        .filter_map(|pair| {
            let (_, producer_name, producer) = pair[0];
            let (_, _, consumer) = pair[1];
            (entries
                .get(producer_name)
                .is_some_and(|entry| entry.references == 1)
                && is_optionless_call(producer, "string")
                && is_numeric_consumer_of(consumer, producer_name))
            .then(|| producer_name.to_string())
        })
        .collect()
}

fn is_optionless_call(expression: &ExpressionNode<'_>, name: &str) -> bool {
    matches!(
        &expression.payload,
        Some(ExpressionPayloadNode::Call(call))
            if call.function.name == name && call.function.options.is_empty()
    )
}

fn is_numeric_consumer_of(expression: &ExpressionNode<'_>, producer: &str) -> bool {
    let Some(ExpressionPayloadNode::Call(call)) = &expression.payload else {
        return false;
    };
    let CallOperandNode::Var(var) = &call.operand else {
        return false;
    };
    matches!(call.function.name, "number" | "integer" | "offset")
        && canonical_reference(var.name).as_deref() == Some(producer)
}

fn declaration_reference_counts(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    doc: &SyntaxDocument<'_>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for declaration in &declarations.inputs {
        count_declaration_references(&declaration.expr.node, &declaration.canonical, &mut counts);
    }
    for declaration in &declarations.locals {
        count_declaration_references(&declaration.expr.node, &declaration.canonical, &mut counts);
    }
    for declaration in &doc.declarations {
        if let Some(DeclarationPayloadNode::Match {
            selectors,
            variants,
        }) = &declaration.payload
        {
            for selector in selectors {
                count_references(selector, &mut counts);
            }
            for variant in variants {
                count_pattern_references(&variant.pattern, &mut counts);
            }
        }
    }
    if let Some(body) = &doc.body {
        let body_source = source.get(body.span.clone()).unwrap_or_default();
        if let Some(pattern) = extract_quoted_pattern(body_source) {
            count_pattern_references(
                &crate::compiler::syntax::parser::parse_pattern(pattern),
                &mut counts,
            );
        } else {
            count_pattern_references(body, &mut counts);
        }
    }
    counts
}

fn count_pattern_references(pattern: &PatternNode<'_>, counts: &mut BTreeMap<String, usize>) {
    for segment in &pattern.segments {
        if let PatternSegmentNode::Expression(expression) = segment {
            count_references(expression, counts);
        }
    }
}

fn count_declaration_references(
    expression: &ExpressionNode<'_>,
    declared_name: &str,
    counts: &mut BTreeMap<String, usize>,
) {
    let mut skipped_operand = false;
    for name in collect_var_refs(expression) {
        if !skipped_operand && name == declared_name {
            skipped_operand = true;
        } else {
            *counts.entry(name).or_default() += 1;
        }
    }
}

fn count_references(expression: &ExpressionNode<'_>, counts: &mut BTreeMap<String, usize>) {
    for name in collect_var_refs(expression) {
        *counts.entry(name).or_default() += 1;
    }
    if let Some(ExpressionPayloadNode::Markup(markup)) = &expression.payload {
        for option in &markup.options {
            if let crate::compiler::syntax::ast::OptionValue::Variable(name) = option.value
                && let Some(name) = canonical_reference(name)
            {
                *counts.entry(name).or_default() += 1;
            }
        }
    }
}

fn canonical_reference(name: &str) -> Option<String> {
    strip_boundary_bidi_controls(name).map(|name| canonicalize_identifier(&name))
}

fn local_literal_kind(source: &str, expr: &ExpressionNode<'_>) -> Option<OperandLiteralKind> {
    let literal = match &expr.payload {
        Some(ExpressionPayloadNode::Literal(literal))
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
