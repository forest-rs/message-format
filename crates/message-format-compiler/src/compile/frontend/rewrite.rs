// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::semantic::SelectorExpr;

use super::bindings::{DeclarationBindings, LocalValue};
use super::local_eval::resolve_alias;

pub(super) fn lower_parts_with_declaration_bindings(
    parts: &mut [Part],
    bindings: &DeclarationBindings,
    repeat_local_pass_after_alias: bool,
) -> Result<(), CompileError> {
    walk_parts_mut(parts, &bindings.locals, &mut |part| {
        lower_part_with_bindings(part, bindings, repeat_local_pass_after_alias)
    })
}

fn lower_part_with_bindings(
    part: &mut Part,
    bindings: &DeclarationBindings,
    repeat_local_pass_after_alias: bool,
) -> Result<(), CompileError> {
    rewrite_dynamic_option_vars_from_locals(part, &bindings.locals);
    match part {
        Part::Var(var) => {
            let canonical = canonicalize_identifier(var);
            if let Some(local) = bindings.locals.get(&canonical) {
                *part = local_value_to_var_part(local, &canonical);
                return Ok(());
            }

            let aliased = resolve_alias(&canonical, &bindings.aliases)?;
            if repeat_local_pass_after_alias && let Some(local) = bindings.locals.get(&aliased) {
                *part = local_value_to_var_part(local, &aliased);
                return Ok(());
            }
            if let Some(function) = bindings
                .local_functions
                .get(&aliased)
                .cloned()
                .or_else(|| bindings.input_functions.get(&aliased).cloned())
            {
                let fb = format!("{{${aliased}}}");
                *part = function.into_part(Some(fb));
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
            if let Some(local) = bindings.locals.get(&canonical) {
                *part = local_value_to_call_part(local, func, &canonical);
                return Ok(());
            }

            let aliased = resolve_alias(&canonical, &bindings.aliases)?;
            if repeat_local_pass_after_alias && let Some(local) = bindings.locals.get(&aliased) {
                *part = local_value_to_call_part(local, func, &aliased);
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

fn local_value_to_var_part(value: &LocalValue, var_name: &str) -> Part {
    match value {
        LocalValue::Literal { value, .. } => Part::Literal(value.clone()),
        LocalValue::UnknownFunction(name) => Part::Call(CallExpr {
            operand: Operand::literal(String::new()),
            func: FunctionSpec {
                name: name.clone(),
                options: Vec::new(),
                origin: None,
            },
            fallback: Some(format!("{{${var_name}}}")),
        }),
    }
}

fn local_value_to_call_part(value: &LocalValue, func: &FunctionSpec, var_name: &str) -> Part {
    match value {
        LocalValue::Literal { value, kind } => Part::Call(CallExpr {
            operand: Operand::Literal {
                value: value.clone(),
                kind: *kind,
            },
            func: func.clone(),
            fallback: Some(format!("{{${var_name}}}")),
        }),
        LocalValue::UnknownFunction(name) => Part::Call(CallExpr {
            operand: Operand::literal(String::new()),
            func: FunctionSpec {
                name: name.clone(),
                options: func.options.clone(),
                origin: func.origin.clone(),
            },
            fallback: Some(format!("{{${var_name}}}")),
        }),
    }
}

fn rewrite_dynamic_option_vars_from_locals(part: &mut Part, locals: &BTreeMap<String, LocalValue>) {
    let func = match part {
        Part::Call(CallExpr { func, .. }) => func,
        _ => return,
    };
    for option in &mut func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = locals.get(var).and_then(LocalValue::as_literal) else {
            continue;
        };
        // Function option literals are stored as decoded semantic text, not raw
        // source tokens, so local substitution can stay allocation-light and
        // avoid reintroducing a quoted-literal protocol here.
        option.value = FunctionOptionValue::Literal(local_value.to_string());
    }
}

fn rewrite_selector_expr_from_locals(
    selector: &mut SelectorExpr,
    locals: &BTreeMap<String, LocalValue>,
) {
    let func = match selector {
        SelectorExpr::Call { func, .. } => func,
        SelectorExpr::Var(_) | SelectorExpr::Literal(_) => return,
    };
    for option in &mut func.options {
        let FunctionOptionValue::Var(var) = &option.value else {
            continue;
        };
        let Some(local_value) = locals.get(var).and_then(LocalValue::as_literal) else {
            continue;
        };
        option.value = FunctionOptionValue::Literal(local_value.to_string());
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
            rewrite_selector_expr_from_locals(selector, locals);
            for arm in arms {
                walk_parts_mut(&mut arm.parts, locals, f)?;
            }
            walk_parts_mut(default, locals, f)?;
        }
    }
    Ok(())
}
