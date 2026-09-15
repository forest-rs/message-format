// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::String,
    vec::Vec,
};

use crate::compiler::semantic::{
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

pub(super) struct CatalogItems {
    pub(super) strings: BTreeSet<String>,
    pub(super) functions: Vec<CollectedFunc>,
    pub(super) function_ids: BTreeMap<FunctionCatalogKey, u16>,
}

pub(super) fn collect_catalog_items(messages: &[Message]) -> Result<CatalogItems, CompileError> {
    let mut items = CatalogItems {
        strings: BTreeSet::new(),
        functions: Vec::new(),
        function_ids: BTreeMap::new(),
    };
    for message in messages {
        items.strings.insert(message.id.clone());
        items.visit_parts(&message.parts)?;
    }
    Ok(items)
}

impl CatalogItems {
    fn visit_parts(&mut self, parts: &[Part]) -> Result<(), CompileError> {
        for part in parts {
            match part {
                Part::Text(_) | Part::Local(_) | Part::CheckSelector(_) => {}
                Part::Literal(value) | Part::Var(value) => {
                    self.strings.insert(value.clone());
                }
                Part::Bind {
                    fallback, value, ..
                } => {
                    self.strings.insert(fallback.clone());
                    self.visit_parts(core::slice::from_ref(value))?;
                }
                Part::Call(call) => self.visit_call(call)?,
                Part::Select(SelectExpr {
                    selector,
                    arms,
                    default,
                }) => {
                    self.visit_selector(selector)?;
                    for arm in arms {
                        self.strings.insert(arm.key.clone());
                        self.visit_parts(&arm.parts)?;
                    }
                    self.visit_parts(default)?;
                }
                Part::MarkupOpen { name, options } | Part::MarkupClose { name, options } => {
                    self.strings.insert(name.clone());
                    self.visit_options(options);
                }
            }
        }
        Ok(())
    }

    fn visit_selector(&mut self, selector: &SelectorExpr) -> Result<(), CompileError> {
        match selector {
            SelectorExpr::Var(name) | SelectorExpr::Literal(name) => {
                self.strings.insert(name.clone());
            }
            SelectorExpr::Call { operand, func } => {
                self.visit_operand(operand)?;
                self.visit_function(func)?;
            }
            SelectorExpr::Local { .. } | SelectorExpr::CheckedLocal { .. } => {}
        }
        Ok(())
    }

    fn visit_call(&mut self, call: &CallExpr) -> Result<(), CompileError> {
        self.visit_operand(&call.operand)?;
        self.visit_function(&call.func)?;
        self.strings.insert(
            call.fallback
                .clone()
                .unwrap_or_else(|| render_call_fallback(&call.operand, &call.func)),
        );
        Ok(())
    }

    fn visit_operand(&mut self, operand: &Operand) -> Result<(), CompileError> {
        match operand {
            Operand::Var(value) | Operand::Literal { value, .. } => {
                self.strings.insert(value.clone());
            }
            Operand::Local(_) => {}
            Operand::Call(call) => self.visit_call(call)?,
        }
        Ok(())
    }

    fn visit_function(&mut self, func: &FunctionSpec) -> Result<(), CompileError> {
        self.strings.insert(func.name.clone());
        self.visit_options(&func.options);
        for option in &func.options {
            if let FunctionOptionValue::Var(var) = &option.value {
                self.strings.insert(format!("{{${var}}}"));
            }
        }
        if !is_optionless_string(func) {
            self.register_function(func)?;
        }
        Ok(())
    }

    fn visit_options(&mut self, options: &[crate::compiler::semantic::FunctionOption]) {
        for option in options {
            self.strings.insert(option.key.clone());
            match &option.value {
                FunctionOptionValue::Literal(value) | FunctionOptionValue::Var(value) => {
                    self.strings.insert(value.clone());
                }
                FunctionOptionValue::ResolvedVar { name, value } => {
                    self.strings.insert(name.clone());
                    self.strings.insert(value.clone());
                }
                FunctionOptionValue::LocalVar { name, .. } => {
                    self.strings.insert(name.clone());
                }
            }
        }
    }

    fn register_function(&mut self, func: &FunctionSpec) -> Result<(), CompileError> {
        let key = function_catalog_key(func);
        if self.function_ids.contains_key(&key) {
            return Ok(());
        }
        let id =
            u16::try_from(self.functions.len()).map_err(|_| CompileError::FunctionIdOverflow)?;
        self.function_ids.insert(key, id);
        let static_options = func
            .options
            .iter()
            .filter_map(|option| match &option.value {
                FunctionOptionValue::Literal(value) => Some((option.key.clone(), value.clone())),
                FunctionOptionValue::Var(_)
                | FunctionOptionValue::ResolvedVar { .. }
                | FunctionOptionValue::LocalVar { .. } => None,
            })
            .collect();
        self.functions.push(CollectedFunc {
            name: func.name.clone(),
            static_options,
        });
        Ok(())
    }
}

fn is_optionless_string(func: &FunctionSpec) -> bool {
    func.name == "string" && func.options.is_empty()
}

fn render_call_fallback(operand: &Operand, func: &FunctionSpec) -> String {
    match operand {
        Operand::Var(var) => format!("{{${var}}}"),
        Operand::Local(slot) => format!("{{<local:{slot}>}}"),
        Operand::Literal { value, .. } if value.is_empty() => format!("{{:{}}}", func.name),
        Operand::Literal { value, .. } => format!("{{|{}|}}", escape_fallback_literal(value)),
        Operand::Call(call) => call
            .fallback
            .clone()
            .unwrap_or_else(|| render_call_fallback(&call.operand, &call.func)),
    }
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
                    FunctionOptionValue::ResolvedVar { name, .. } => {
                        FunctionCatalogOptionValue::Var(name.clone())
                    }
                    FunctionOptionValue::LocalVar { name, .. } => {
                        FunctionCatalogOptionValue::Var(name.clone())
                    }
                },
            })
            .collect(),
    }
}

pub(super) fn escape_fallback_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

pub(super) fn function_dynamic_options(
    func: &FunctionSpec,
) -> Vec<(&str, &str, Option<u32>, Option<&str>)> {
    let mut out = Vec::new();
    for option in &func.options {
        match &option.value {
            FunctionOptionValue::Var(var) => {
                out.push((option.key.as_str(), var.as_str(), None, None));
            }
            FunctionOptionValue::ResolvedVar { name, value } => {
                out.push((
                    option.key.as_str(),
                    name.as_str(),
                    None,
                    Some(value.as_str()),
                ));
            }
            FunctionOptionValue::LocalVar { name, slot } => {
                out.push((option.key.as_str(), name.as_str(), Some(*slot), None));
            }
            FunctionOptionValue::Literal(_) => {}
        }
    }
    out
}
