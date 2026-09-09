// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec::Vec,
};

use super::*;
use crate::compiler::semantic::SelectorExpr;

use super::bindings::{DeclFunction, DeclarationBindings, LocalValue};
use super::local_eval::resolve_alias;

pub(super) fn lower_parts_with_declaration_bindings(
    parts: &mut [Part],
    bindings: &DeclarationBindings,
    repeat_local_pass_after_alias: bool,
) -> Result<(), CompileError> {
    walk_parts_mut(parts, &bindings.locals, &mut |part| {
        lower_part_with_bindings(part, bindings, repeat_local_pass_after_alias, None)
    })
}

pub(super) fn lower_declaration_prelude(
    source: &str,
    declarations: &crate::compiler::syntax::semantic::CanonicalDeclarationPrelude<'_>,
    bindings: &DeclarationBindings,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Vec<Part>, CompileError> {
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

    let mut parts = Vec::with_capacity(ordered.len());
    for (_, name, expression) in ordered {
        let Some(slot) = bindings.slots.get(name).copied() else {
            continue;
        };
        let mut value = lower_expression_node_to_part(source, expression, ctx, function_origin)?;
        if let Part::Call(call) = &mut value {
            call.fallback = Some(format!("{{${name}}}"));
        }
        lower_part_with_bindings(&mut value, bindings, false, Some(name))?;
        parts.push(Part::Bind {
            slot,
            fallback: format!("{{${name}}}"),
            value: Box::new(value),
        });
    }
    Ok(parts)
}

/// Remove declaration slots that are provably unobservable and then restore
/// the dense slot numbering required by the catalog verifier.
///
/// Aliases and optionless built-in string resolutions that no remaining
/// expression reads do not consume VM storage. Other calls remain observable
/// because a host can report diagnostics from them.
pub(super) fn compact_declaration_slots(
    declarations: &mut Vec<Part>,
    body: &mut [Part],
) -> Result<(), CompileError> {
    let inlined_string_slots = inline_adjacent_string_numeric_chains(declarations, body);
    let mut live = BTreeSet::new();
    collect_part_slots(body, &mut live);
    let mut retain = vec![true; declarations.len()];
    for (index, declaration) in declarations.iter().enumerate().rev() {
        let Part::Bind { slot, value, .. } = declaration else {
            continue;
        };
        if !live.contains(slot)
            && (binding_is_elidable(declaration) || inlined_string_slots.contains(slot))
        {
            retain[index] = false;
            continue;
        }
        collect_part_slots(core::slice::from_ref(value.as_ref()), &mut live);
    }

    let mut remap = BTreeMap::new();
    let mut next_slot = 0_u32;
    for (declaration, keep) in declarations.iter().zip(&retain) {
        if !keep {
            continue;
        }
        if let Part::Bind { slot, .. } = declaration {
            remap.insert(*slot, next_slot);
            next_slot = next_slot
                .checked_add(1)
                .ok_or_else(|| CompileError::size_overflow("declaration slots"))?;
        }
    }

    let mut index = 0_usize;
    declarations.retain(|_| {
        let keep = retain[index];
        index += 1;
        keep
    });
    remap_part_slots(declarations, &remap)?;
    remap_part_slots(body, &remap)
}

fn inline_adjacent_string_numeric_chains(
    declarations: &mut [Part],
    body: &[Part],
) -> BTreeSet<u32> {
    let candidates = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            let Part::Bind { slot, value, .. } = declaration else {
                return None;
            };
            let Part::Call(call) = value.as_ref() else {
                return None;
            };
            (call.func.name == "string" && call.func.options.is_empty())
                .then(|| (index, *slot, call.clone()))
        })
        .collect::<Vec<_>>();
    let mut inlined_slots = BTreeSet::new();

    for (index, slot, string_call) in candidates {
        let uses = declarations
            .iter()
            .map(|part| count_slot_uses(part, slot))
            .sum::<usize>()
            + body
                .iter()
                .map(|part| count_slot_uses(part, slot))
                .sum::<usize>();
        if uses != 1 {
            continue;
        }
        let Some(Part::Bind { value, .. }) = declarations.get_mut(index + 1) else {
            continue;
        };
        let Part::Call(consumer) = value.as_mut() else {
            continue;
        };
        if !matches!(consumer.func.name.as_str(), "number" | "integer" | "offset")
            || !matches!(consumer.operand, Operand::Local(candidate) if candidate == slot)
        {
            continue;
        }
        consumer.operand = Operand::Call(Box::new(string_call));
        inlined_slots.insert(slot);
    }
    inlined_slots
}

fn count_slot_uses(part: &Part, target: u32) -> usize {
    match part {
        Part::Local(slot) | Part::CheckSelector(slot) => usize::from(*slot == target),
        Part::Call(call) => {
            count_operand_slot_uses(&call.operand, target)
                + call
                    .func
                    .options
                    .iter()
                    .filter(|option| {
                        matches!(
                            option.value,
                            FunctionOptionValue::LocalVar { slot, .. } if slot == target
                        )
                    })
                    .count()
        }
        Part::Select(select) => {
            count_selector_slot_uses(&select.selector, target)
                + select
                    .arms
                    .iter()
                    .flat_map(|arm| &arm.parts)
                    .map(|part| count_slot_uses(part, target))
                    .sum::<usize>()
                + select
                    .default
                    .iter()
                    .map(|part| count_slot_uses(part, target))
                    .sum::<usize>()
        }
        Part::Bind { value, .. } => count_slot_uses(value, target),
        Part::MarkupOpen { options, .. } | Part::MarkupClose { options, .. } => options
            .iter()
            .filter(|option| {
                matches!(
                    option.value,
                    FunctionOptionValue::LocalVar { slot, .. } if slot == target
                )
            })
            .count(),
        Part::Text(_) | Part::Literal(_) | Part::Var(_) => 0,
    }
}

fn count_selector_slot_uses(selector: &SelectorExpr, target: u32) -> usize {
    match selector {
        SelectorExpr::Local { slot, .. } | SelectorExpr::CheckedLocal { slot, .. } => {
            usize::from(*slot == target)
        }
        SelectorExpr::Call { operand, func } => {
            count_operand_slot_uses(operand, target)
                + func
                    .options
                    .iter()
                    .filter(|option| {
                        matches!(
                            option.value,
                            FunctionOptionValue::LocalVar { slot, .. } if slot == target
                        )
                    })
                    .count()
        }
        SelectorExpr::Var(_) | SelectorExpr::Literal(_) => 0,
    }
}

fn count_operand_slot_uses(operand: &Operand, target: u32) -> usize {
    match operand {
        Operand::Local(slot) => usize::from(*slot == target),
        Operand::Call(call) => {
            count_operand_slot_uses(&call.operand, target)
                + call
                    .func
                    .options
                    .iter()
                    .filter(|option| {
                        matches!(
                            option.value,
                            FunctionOptionValue::LocalVar { slot, .. } if slot == target
                        )
                    })
                    .count()
        }
        Operand::Var(_) | Operand::Literal { .. } => 0,
    }
}

fn binding_is_elidable(binding: &Part) -> bool {
    let Part::Bind { value, .. } = binding else {
        return false;
    };
    matches!(value.as_ref(), Part::Local(_))
}

pub(super) fn collect_part_slots(parts: &[Part], slots: &mut BTreeSet<u32>) {
    for part in parts {
        match part {
            Part::Local(slot) | Part::CheckSelector(slot) => {
                slots.insert(*slot);
            }
            Part::Call(call) => collect_call_slots(call, slots),
            Part::Select(select) => {
                collect_selector_slots(&select.selector, slots);
                for arm in &select.arms {
                    collect_part_slots(&arm.parts, slots);
                }
                collect_part_slots(&select.default, slots);
            }
            Part::Bind { value, .. } => collect_part_slots(core::slice::from_ref(value), slots),
            Part::MarkupOpen { options, .. } | Part::MarkupClose { options, .. } => {
                collect_option_slots(options, slots);
            }
            Part::Text(_) | Part::Literal(_) | Part::Var(_) => {}
        }
    }
}

fn collect_selector_slots(selector: &SelectorExpr, slots: &mut BTreeSet<u32>) {
    match selector {
        SelectorExpr::Local { slot, .. } | SelectorExpr::CheckedLocal { slot, .. } => {
            slots.insert(*slot);
        }
        SelectorExpr::Call { operand, func } => {
            collect_operand_slots(operand, slots);
            collect_option_slots(&func.options, slots);
        }
        SelectorExpr::Var(_) | SelectorExpr::Literal(_) => {}
    }
}

fn collect_call_slots(call: &CallExpr, slots: &mut BTreeSet<u32>) {
    collect_operand_slots(&call.operand, slots);
    collect_option_slots(&call.func.options, slots);
}

fn collect_operand_slots(operand: &Operand, slots: &mut BTreeSet<u32>) {
    match operand {
        Operand::Local(slot) => {
            slots.insert(*slot);
        }
        Operand::Call(call) => collect_call_slots(call, slots),
        Operand::Var(_) | Operand::Literal { .. } => {}
    }
}

fn collect_option_slots(options: &[FunctionOption], slots: &mut BTreeSet<u32>) {
    for option in options {
        if let FunctionOptionValue::LocalVar { slot, .. } = &option.value {
            slots.insert(*slot);
        }
    }
}

fn remap_part_slots(parts: &mut [Part], remap: &BTreeMap<u32, u32>) -> Result<(), CompileError> {
    for part in parts {
        match part {
            Part::Local(slot) | Part::CheckSelector(slot) => remap_slot(slot, remap)?,
            Part::Call(call) => remap_call_slots(call, remap)?,
            Part::Select(select) => {
                remap_selector_slots(&mut select.selector, remap)?;
                for arm in &mut select.arms {
                    remap_part_slots(&mut arm.parts, remap)?;
                }
                remap_part_slots(&mut select.default, remap)?;
            }
            Part::Bind { slot, value, .. } => {
                remap_slot(slot, remap)?;
                remap_part_slots(core::slice::from_mut(value), remap)?;
            }
            Part::MarkupOpen { options, .. } | Part::MarkupClose { options, .. } => {
                remap_option_slots(options, remap)?;
            }
            Part::Text(_) | Part::Literal(_) | Part::Var(_) => {}
        }
    }
    Ok(())
}

fn remap_selector_slots(
    selector: &mut SelectorExpr,
    remap: &BTreeMap<u32, u32>,
) -> Result<(), CompileError> {
    match selector {
        SelectorExpr::Local { slot, .. } | SelectorExpr::CheckedLocal { slot, .. } => {
            remap_slot(slot, remap)
        }
        SelectorExpr::Call { operand, func } => {
            remap_operand_slots(operand, remap)?;
            remap_option_slots(&mut func.options, remap)
        }
        SelectorExpr::Var(_) | SelectorExpr::Literal(_) => Ok(()),
    }
}

fn remap_call_slots(call: &mut CallExpr, remap: &BTreeMap<u32, u32>) -> Result<(), CompileError> {
    remap_operand_slots(&mut call.operand, remap)?;
    remap_option_slots(&mut call.func.options, remap)
}

fn remap_operand_slots(
    operand: &mut Operand,
    remap: &BTreeMap<u32, u32>,
) -> Result<(), CompileError> {
    match operand {
        Operand::Local(slot) => remap_slot(slot, remap),
        Operand::Call(call) => remap_call_slots(call, remap),
        Operand::Var(_) | Operand::Literal { .. } => Ok(()),
    }
}

fn remap_option_slots(
    options: &mut [FunctionOption],
    remap: &BTreeMap<u32, u32>,
) -> Result<(), CompileError> {
    for option in options {
        if let FunctionOptionValue::LocalVar { slot, .. } = &mut option.value {
            remap_slot(slot, remap)?;
        }
    }
    Ok(())
}

fn remap_slot(slot: &mut u32, remap: &BTreeMap<u32, u32>) -> Result<(), CompileError> {
    *slot = *remap
        .get(slot)
        .ok_or_else(|| CompileError::internal("reference to removed declaration slot"))?;
    Ok(())
}

fn lower_part_with_bindings(
    part: &mut Part,
    bindings: &DeclarationBindings,
    repeat_local_pass_after_alias: bool,
    excluded: Option<&str>,
) -> Result<(), CompileError> {
    rewrite_dynamic_option_vars_from_locals(part, &bindings.locals, &bindings.slots);
    match part {
        Part::Var(var) => {
            let canonical = canonicalize_identifier(var);
            let _ = resolve_alias(&canonical, &bindings.aliases)?;
            if excluded == Some(canonical.as_str()) {
                return Ok(());
            }
            if excluded != Some(canonical.as_str())
                && slot_is_runtime(bindings, &canonical)
                && let Some(slot) = bindings.slots.get(&canonical)
            {
                *part = Part::Local(*slot);
                return Ok(());
            }
            if let Some(LocalValue::Literal { value, .. }) = bindings.locals.get(&canonical) {
                *part = Part::Literal(value.clone());
                return Ok(());
            }

            let aliased = resolve_alias(&canonical, &bindings.aliases)?;
            if excluded != Some(aliased.as_str())
                && slot_is_runtime(bindings, &aliased)
                && let Some(slot) = bindings.slots.get(&aliased)
            {
                *part = Part::Local(*slot);
                return Ok(());
            }
            if repeat_local_pass_after_alias
                && let Some(LocalValue::Literal { value, .. }) = bindings.locals.get(&aliased)
            {
                *part = Part::Literal(value.clone());
                return Ok(());
            }
            if let Some(function) = bindings.input_functions.get(&aliased).cloned() {
                let fb = format!("{{${aliased}}}");
                let mut lowered = function.into_part(Some(fb));
                rewrite_dynamic_option_vars_from_locals(
                    &mut lowered,
                    &bindings.locals,
                    &bindings.slots,
                );
                *part = lowered;
                return Ok(());
            }
            if let Some(declared) = bindings.input_aliases.get(&aliased) {
                *var = declared.clone();
            } else {
                *var = aliased;
            }
        }
        Part::Call(CallExpr {
            operand: Operand::Var(var),
            func,
            fallback,
        }) => {
            let retained_fallback = fallback.clone();
            let canonical = canonicalize_identifier(var);
            let _ = resolve_alias(&canonical, &bindings.aliases)?;
            if excluded == Some(canonical.as_str()) {
                return Ok(());
            }
            if excluded != Some(canonical.as_str())
                && slot_is_runtime(bindings, &canonical)
                && let Some(slot) = bindings.slots.get(&canonical)
            {
                *part = Part::Call(CallExpr {
                    operand: Operand::Local(*slot),
                    func: func.clone(),
                    fallback: retained_fallback
                        .clone()
                        .or_else(|| Some(format!("{{${canonical}}}"))),
                });
                return Ok(());
            }
            if let Some(LocalValue::Literal { value, kind }) = bindings.locals.get(&canonical) {
                *part = Part::Call(CallExpr {
                    operand: Operand::Literal {
                        value: value.clone(),
                        kind: *kind,
                    },
                    func: func.clone(),
                    fallback: retained_fallback
                        .clone()
                        .or_else(|| Some(format!("{{${canonical}}}"))),
                });
                return Ok(());
            }

            let aliased = resolve_alias(&canonical, &bindings.aliases)?;
            if excluded != Some(aliased.as_str())
                && slot_is_runtime(bindings, &aliased)
                && let Some(slot) = bindings.slots.get(&aliased)
            {
                *part = Part::Call(CallExpr {
                    operand: Operand::Local(*slot),
                    func: func.clone(),
                    fallback: retained_fallback
                        .clone()
                        .or_else(|| Some(format!("{{${aliased}}}"))),
                });
                return Ok(());
            }
            if repeat_local_pass_after_alias
                && let Some(LocalValue::Literal { value, kind }) = bindings.locals.get(&aliased)
            {
                *part = Part::Call(CallExpr {
                    operand: Operand::Literal {
                        value: value.clone(),
                        kind: *kind,
                    },
                    func: func.clone(),
                    fallback: retained_fallback
                        .clone()
                        .or_else(|| Some(format!("{{${aliased}}}"))),
                });
                return Ok(());
            }
            if let Some(function) = bindings.input_functions.get(&aliased).cloned() {
                let mut lowered = Part::Call(CallExpr {
                    operand: input_function_operand(function),
                    func: func.clone(),
                    fallback: retained_fallback
                        .clone()
                        .or_else(|| Some(format!("{{${aliased}}}"))),
                });
                rewrite_dynamic_option_vars_from_locals(
                    &mut lowered,
                    &bindings.locals,
                    &bindings.slots,
                );
                *part = lowered;
                return Ok(());
            }
            if let Some(declared) = bindings.input_aliases.get(&aliased) {
                *var = declared.clone();
            } else {
                *var = aliased;
            }
        }
        _ => {}
    }
    Ok(())
}

fn slot_is_runtime(bindings: &DeclarationBindings, name: &str) -> bool {
    bindings
        .locals
        .get(name)
        .is_none_or(|value| value.as_literal().is_none())
}

fn input_function_operand(function: DeclFunction) -> Operand {
    Operand::Call(Box::new(CallExpr {
        operand: function.operand,
        func: function.func,
        fallback: None,
    }))
}

fn rewrite_dynamic_option_vars_from_locals(
    part: &mut Part,
    locals: &BTreeMap<String, LocalValue>,
    slots: &BTreeMap<String, u32>,
) {
    let Part::Call(call) = part else { return };
    rewrite_call_options_from_locals(call, locals, slots);
}

fn rewrite_call_options_from_locals(
    call: &mut CallExpr,
    locals: &BTreeMap<String, LocalValue>,
    slots: &BTreeMap<String, u32>,
) {
    let func = &mut call.func;
    for option in &mut func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = locals.get(var).and_then(LocalValue::as_literal) else {
            if let Some(slot) = slots.get(var) {
                option.value = FunctionOptionValue::LocalVar {
                    name: var.clone(),
                    slot: *slot,
                };
            }
            continue;
        };
        // Preserve the variable origin while carrying the local payload;
        // local declarations are not message arguments, but option values
        // still need their dynamic fallback semantics.
        option.value = FunctionOptionValue::ResolvedVar {
            name: var.clone(),
            value: local_value.to_string(),
        };
    }
    if let Operand::Call(nested) = &mut call.operand {
        rewrite_call_options_from_locals(nested, locals, slots);
    }
}

pub(super) fn rewrite_selector_expr_from_locals(
    selector: &mut SelectorExpr,
    locals: &BTreeMap<String, LocalValue>,
    slots: &BTreeMap<String, u32>,
) {
    let (operand, func) = match selector {
        SelectorExpr::Call { operand, func } => (operand, func),
        SelectorExpr::Var(_)
        | SelectorExpr::Local { .. }
        | SelectorExpr::CheckedLocal { .. }
        | SelectorExpr::Literal(_) => return,
    };
    for option in &mut func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = locals.get(var).and_then(LocalValue::as_literal) else {
            continue;
        };
        option.value = FunctionOptionValue::ResolvedVar {
            name: var.clone(),
            value: local_value.to_string(),
        };
    }
    if let Operand::Call(nested) = operand {
        rewrite_call_options_from_locals(nested, locals, slots);
    }
}

fn walk_parts_mut(
    parts: &mut [Part],
    locals: &BTreeMap<String, LocalValue>,
    f: &mut impl FnMut(&mut Part) -> Result<(), CompileError>,
) -> Result<(), CompileError> {
    for part in parts {
        f(part)?;
        if let Part::Select(SelectExpr {
            selector,
            arms,
            default,
        }) = part
        {
            rewrite_selector_expr_from_locals(selector, locals, &BTreeMap::new());
            for arm in arms {
                walk_parts_mut(&mut arm.parts, locals, f)?;
            }
            walk_parts_mut(default, locals, f)?;
        }
    }
    Ok(())
}
