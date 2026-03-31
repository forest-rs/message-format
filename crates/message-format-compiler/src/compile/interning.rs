// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::{BTreeMap, BTreeSet};

use crate::semantic::{
    CallExpr, FunctionOptionValue, FunctionSpec, Message, Operand, Part, SelectExpr, SelectorExpr,
};

use super::CompileError;

/// A collected function entry for the FUNC chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CollectedFunc {
    pub name: String,
    pub static_options: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FunctionCatalogKey {
    name: String,
    options: Vec<FunctionCatalogOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FunctionCatalogOption {
    key: String,
    value: FunctionCatalogOptionValue,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FunctionCatalogOptionValue {
    Literal(String),
    Var(String),
}

pub(super) fn collect_strings(messages: &[Message], out: &mut BTreeSet<String>) {
    for message in messages {
        out.insert(message.id.clone());
        collect_parts_strings(&message.parts, out);
    }
}

fn collect_parts_strings(parts: &[Part], out: &mut BTreeSet<String>) {
    for part in parts {
        match part {
            Part::Text(_) | Part::Literal(_) => {}
            Part::Var(name) => {
                out.insert(name.clone());
            }
            Part::Call(CallExpr {
                operand,
                func,
                fallback,
            }) => {
                collect_operand_strings(operand, out);
                collect_function_strings(func, out);
                if let Some(fb) = fallback {
                    out.insert(fb.clone());
                } else {
                    out.insert(render_call_fallback(operand, func));
                }
            }
            Part::Select(SelectExpr {
                selector,
                arms,
                default,
            }) => {
                collect_selector_strings(selector, out);
                for arm in arms {
                    out.insert(arm.key.clone());
                    collect_parts_strings(&arm.parts, out);
                }
                collect_parts_strings(default, out);
            }
            Part::MarkupOpen { name, options } | Part::MarkupClose { name, options } => {
                out.insert(name.clone());
                for option in options {
                    out.insert(option.key.clone());
                    match &option.value {
                        FunctionOptionValue::Literal(value) => {
                            out.insert(value.clone());
                        }
                        FunctionOptionValue::Var(var) => {
                            out.insert(var.clone());
                        }
                    }
                }
            }
        }
    }
}

/// Insert function name and static option key/value strings into the pool,
/// plus dynamic option key and variable strings.
fn collect_function_strings(func: &FunctionSpec, out: &mut BTreeSet<String>) {
    out.insert(func.name.clone());
    for option in &func.options {
        out.insert(option.key.clone());
        match &option.value {
            FunctionOptionValue::Literal(value) => {
                out.insert(value.clone());
            }
            FunctionOptionValue::Var(var) => {
                out.insert(var.clone());
            }
        }
    }
}

/// Collect deduplicated function entries from all messages, returning the
/// entries and a map from catalog key to `func_id`.
pub(super) fn collect_functions(
    messages: &[Message],
) -> Result<(Vec<CollectedFunc>, BTreeMap<FunctionCatalogKey, u16>), CompileError> {
    let mut func_map: BTreeMap<FunctionCatalogKey, u16> = BTreeMap::new();
    let mut entries: Vec<CollectedFunc> = Vec::new();

    for message in messages {
        collect_parts_functions(&message.parts, &mut func_map, &mut entries)?;
    }

    Ok((entries, func_map))
}

fn collect_parts_functions(
    parts: &[Part],
    func_map: &mut BTreeMap<FunctionCatalogKey, u16>,
    entries: &mut Vec<CollectedFunc>,
) -> Result<(), CompileError> {
    for part in parts {
        match part {
            Part::Call(CallExpr { func, .. }) => {
                register_function(func, func_map, entries)?;
            }
            Part::Select(SelectExpr {
                selector,
                arms,
                default,
            }) => {
                collect_selector_functions(selector, func_map, entries)?;
                for arm in arms {
                    collect_parts_functions(&arm.parts, func_map, entries)?;
                }
                collect_parts_functions(default, func_map, entries)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn collect_selector_strings(selector: &SelectorExpr, out: &mut BTreeSet<String>) {
    match selector {
        SelectorExpr::Var(name) => {
            out.insert(name.clone());
        }
        SelectorExpr::Call { operand, func } => {
            collect_operand_strings(operand, out);
            collect_function_strings(func, out);
        }
        SelectorExpr::Literal(value) => {
            out.insert(value.clone());
        }
    }
}

fn collect_selector_functions(
    selector: &SelectorExpr,
    func_map: &mut BTreeMap<FunctionCatalogKey, u16>,
    entries: &mut Vec<CollectedFunc>,
) -> Result<(), CompileError> {
    match selector {
        SelectorExpr::Call { func, .. } => register_function(func, func_map, entries),
        SelectorExpr::Var(_) | SelectorExpr::Literal(_) => Ok(()),
    }
}

fn collect_operand_strings(operand: &Operand, out: &mut BTreeSet<String>) {
    match operand {
        Operand::Var(value) | Operand::Literal { value, .. } => {
            out.insert(value.clone());
        }
    }
}

fn render_call_fallback(operand: &Operand, func: &FunctionSpec) -> String {
    match operand {
        Operand::Var(var) => format!("{{${var}}}"),
        Operand::Literal { value, .. } if value.is_empty() => format!("{{:{}}}", func.name),
        Operand::Literal { value, .. } => format!("{{|{}|}}", escape_fallback_literal(value)),
    }
}

fn register_function(
    func: &FunctionSpec,
    func_map: &mut BTreeMap<FunctionCatalogKey, u16>,
    entries: &mut Vec<CollectedFunc>,
) -> Result<(), CompileError> {
    let key = function_catalog_key(func);
    if func_map.contains_key(&key) {
        return Ok(());
    }
    let id = u16::try_from(entries.len()).map_err(|_| CompileError::FunctionIdOverflow)?;
    func_map.insert(key, id);

    let static_options: Vec<(String, String)> = func
        .options
        .iter()
        .filter_map(|opt| match &opt.value {
            FunctionOptionValue::Literal(value) => Some((opt.key.clone(), value.clone())),
            FunctionOptionValue::Var(_) => None,
        })
        .collect();

    entries.push(CollectedFunc {
        name: func.name.clone(),
        static_options,
    });
    Ok(())
}

pub(crate) fn function_catalog_key(func: &FunctionSpec) -> FunctionCatalogKey {
    FunctionCatalogKey {
        name: func.name.clone(),
        options: func
            .options
            .iter()
            .map(|option| FunctionCatalogOption {
                key: option.key.clone(),
                value: match &option.value {
                    FunctionOptionValue::Literal(value) => {
                        FunctionCatalogOptionValue::Literal(value.clone())
                    }
                    FunctionOptionValue::Var(var) => FunctionCatalogOptionValue::Var(var.clone()),
                },
            })
            .collect(),
    }
}

pub(super) fn escape_fallback_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

pub(super) fn function_dynamic_options(func: &FunctionSpec) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for option in &func.options {
        if let FunctionOptionValue::Var(var) = &option.value {
            out.push((option.key.as_str(), var.as_str()));
        }
    }
    out
}
