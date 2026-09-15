// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use crate::compiler::semantic::{
    CallExpr, FunctionOption, FunctionOptionValue, Operand, Part, SelectExpr, SelectorExpr,
};

/// How declaration results are consumed after eager evaluation.
#[derive(Clone, Copy, Default)]
pub(super) struct DeclarationUse {
    format: bool,
    select: bool,
}

impl DeclarationUse {
    pub(super) fn formats(self) -> bool {
        self.format
    }

    pub(super) fn selects(self) -> bool {
        self.select
    }

    fn merge(&mut self, other: Self) -> bool {
        let previous = *self;
        self.format |= other.format;
        self.select |= other.select;
        self.format != previous.format || self.select != previous.select
    }
}

/// One analysis of declaration references shared by validation and rewriting.
#[derive(Default)]
pub(super) struct DeclarationUses {
    slots: BTreeMap<u32, SlotAnalysis>,
}

#[derive(Default)]
struct SlotAnalysis {
    use_kind: DeclarationUse,
    references: usize,
}

impl DeclarationUses {
    pub(super) fn analyze(parts: &[Part]) -> Self {
        Self::analyze_slices(&[parts])
    }

    pub(super) fn analyze_pair(first: &[Part], second: &[Part]) -> Self {
        Self::analyze_slices(&[first, second])
    }

    fn analyze_slices(slices: &[&[Part]]) -> Self {
        let mut analysis = Self::default();
        let mut aliases = Vec::new();
        for parts in slices {
            analysis.visit_parts(parts, &mut aliases);
        }

        while aliases.iter().fold(false, |changed, &(target, source)| {
            let use_kind = analysis.use_of(target);
            analysis.mark(source, use_kind) | changed
        }) {}

        analysis
    }

    pub(super) fn use_of(&self, slot: u32) -> DeclarationUse {
        self.slots
            .get(&slot)
            .map(|analysis| analysis.use_kind)
            .unwrap_or_default()
    }

    pub(super) fn reference_count(&self, slot: u32) -> usize {
        self.slots
            .get(&slot)
            .map_or(0, |analysis| analysis.references)
    }

    pub(super) fn referenced_slots(&self) -> BTreeSet<u32> {
        self.slots
            .iter()
            .filter_map(|(slot, analysis)| (analysis.references != 0).then_some(*slot))
            .collect()
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
                        self.count_reference(*source);
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
        self.count_reference(slot);
    }

    fn mark(&mut self, slot: u32, use_kind: DeclarationUse) -> bool {
        self.slots.entry(slot).or_default().use_kind.merge(use_kind)
    }

    fn count_reference(&mut self, slot: u32) {
        self.slots.entry(slot).or_default().references += 1;
    }
}

impl DeclarationUse {
    const fn format() -> Self {
        Self {
            format: true,
            select: false,
        }
    }

    const fn select() -> Self {
        Self {
            format: false,
            select: true,
        }
    }
}
