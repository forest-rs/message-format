// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
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
        if let Part::Call(CallExpr {
            operand: Operand::Var(var),
            func,
            ..
        }) = &value
            && declarations
                .inputs
                .iter()
                .any(|decl| decl.canonical == name)
            && func.name == "string"
            && func.options.is_empty()
        {
            value = Part::Var(var.clone());
        }
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
            ..
        }) => {
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
                    fallback: Some(format!("{{${canonical}}}")),
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
                    fallback: Some(format!("{{${canonical}}}")),
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
                    fallback: Some(format!("{{${aliased}}}")),
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
                    fallback: Some(format!("{{${aliased}}}")),
                });
                return Ok(());
            }
            if let Some(function) = bindings.input_functions.get(&aliased).cloned() {
                let mut lowered = Part::Call(CallExpr {
                    operand: input_function_operand(function),
                    func: func.clone(),
                    fallback: Some(format!("{{${aliased}}}")),
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
    !bindings
        .locals
        .get(name)
        .is_some_and(|value| value.as_literal().is_some())
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
        if option.key == "select" {
            // Keep the option dynamic so invalid values are diagnosed by the
            // builtin at runtime, while carrying the local payload because a
            // local declaration is not a message argument.
            option.value = FunctionOptionValue::ResolvedVar {
                name: var.clone(),
                value: local_value.to_string(),
            };
            continue;
        }
        // Function option literals are stored as decoded semantic text, not raw
        // source tokens, so local substitution can stay allocation-light and
        // avoid reintroducing a quoted-literal protocol here.
        option.value = FunctionOptionValue::Literal(local_value.to_string());
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
        SelectorExpr::Var(_) | SelectorExpr::Local { .. } | SelectorExpr::Literal(_) => return,
    };
    for option in &mut func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = locals.get(var).and_then(LocalValue::as_literal) else {
            continue;
        };
        if option.key == "select" {
            option.value = FunctionOptionValue::ResolvedVar {
                name: var.clone(),
                value: local_value.to_string(),
            };
            continue;
        }
        option.value = FunctionOptionValue::Literal(local_value.to_string());
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
