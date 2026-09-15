// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{collections::BTreeMap, vec::Vec};

use crate::compiler::semantic::{
    CallExpr, FunctionOption, FunctionOptionValue, Operand, Part, SelectExpr, SelectorExpr,
};

/// How declaration results are consumed after eager evaluation.
#[derive(Clone, Copy, Default)]
pub(super) struct DeclarationUse(u8);

impl DeclarationUse {
    pub(super) fn formats(self) -> bool {
        self.0 & Self::format().0 != 0
    }

    pub(super) fn selects(self) -> bool {
        self.0 & Self::select().0 != 0
    }

    fn merge(&mut self, other: Self) -> bool {
        let previous = *self;
        self.0 |= other.0;
        self.0 != previous.0
    }
}

/// How declaration results are used, including uses propagated through aliases.
#[derive(Default)]
pub(super) struct DeclarationUses {
    slots: BTreeMap<u32, DeclarationUse>,
}

impl DeclarationUses {
    pub(super) fn analyze(parts: &[Part]) -> Self {
        let mut analysis = Self::default();
        let mut aliases = Vec::new();
        analysis.visit_parts(parts, &mut aliases);

        while aliases.iter().fold(false, |changed, &(target, source)| {
            let use_kind = analysis.use_of(target);
            analysis.mark(source, use_kind) | changed
        }) {}

        analysis
    }

    pub(super) fn use_of(&self, slot: u32) -> DeclarationUse {
        self.slots.get(&slot).copied().unwrap_or_default()
    }

    fn visit_parts(&mut self, parts: &[Part], aliases: &mut Vec<(u32, u32)>) {
        for part in parts {
            match part {
                Part::Local(slot) => self.record(*slot, DeclarationUse::format()),
                Part::CheckSelector(slot) => {
                    self.record(*slot, DeclarationUse::select());
                }
                Part::Call(call) => self.visit_call_inputs(call),
                Part::Bind { slot, value, .. } => {
                    if let Part::Local(source) = value.as_ref() {
                        aliases.push((*slot, *source));
                    } else {
                        self.visit_parts(core::slice::from_ref(value), aliases);
                    }
                }
                Part::Select(SelectExpr {
                    selector,
                    arms,
                    default,
                }) => {
                    self.visit_selector(selector);
                    for arm in arms {
                        self.visit_parts(&arm.parts, aliases);
                    }
                    self.visit_parts(default, aliases);
                }
                Part::MarkupOpen { options, .. } | Part::MarkupClose { options, .. } => {
                    self.visit_options(options);
                }
                Part::Text(_) | Part::Literal(_) | Part::Var(_) => {}
            }
        }
    }

    fn visit_selector(&mut self, selector: &SelectorExpr) {
        match selector {
            SelectorExpr::Local { slot, .. } | SelectorExpr::CheckedLocal { slot, .. } => {
                self.record(*slot, DeclarationUse::select());
            }
            SelectorExpr::Call { operand, func } => {
                self.visit_operand(operand);
                self.visit_options(&func.options);
            }
            SelectorExpr::Var(_) | SelectorExpr::Literal(_) => {}
        }
    }

    fn visit_call_inputs(&mut self, call: &CallExpr) {
        self.visit_operand(&call.operand);
        self.visit_options(&call.func.options);
    }

    fn visit_operand(&mut self, operand: &Operand) {
        match operand {
            Operand::Local(slot) => {
                self.record(*slot, DeclarationUse::format());
            }
            Operand::Call(call) => self.visit_call_inputs(call),
            Operand::Var(_) | Operand::Literal { .. } => {}
        }
    }

    fn visit_options(&mut self, options: &[FunctionOption]) {
        for option in options {
            if let FunctionOptionValue::LocalVar { slot, .. } = option.value {
                self.record(slot, DeclarationUse::format());
            }
        }
    }

    fn record(&mut self, slot: u32, use_kind: DeclarationUse) {
        self.mark(slot, use_kind);
    }

    fn mark(&mut self, slot: u32, use_kind: DeclarationUse) -> bool {
        self.slots.entry(slot).or_default().merge(use_kind)
    }
}

impl DeclarationUse {
    const fn format() -> Self {
        Self(1)
    }

    const fn select() -> Self {
        Self(2)
    }
}
