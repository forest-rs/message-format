// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::semantic::{SelectorExpr, SourceSpan};
use crate::syntax::literal::parse_number_literal;

#[derive(Clone)]
pub(super) struct MatchKey {
    pub(super) key: String,
    pub(super) is_default: bool,
    pub(super) span: std::ops::Range<usize>,
    pub(super) origin: Option<SourceSpan>,
}

#[derive(Clone)]
pub(super) struct MatchArm<'a> {
    pub(super) keys: Vec<MatchKey>,
    pub(super) pattern: crate::syntax::ast::PatternNode<'a>,
}

#[derive(Clone)]
pub(super) struct LoweredMatchArm {
    pub(super) keys: Vec<MatchKey>,
    pub(super) parts: Vec<Part>,
}

impl LoweredMatchArm {
    pub(super) fn is_default(&self) -> bool {
        self.keys.iter().all(|key| key.is_default)
    }

    pub(super) fn matches(&self, selectors: &[String]) -> bool {
        if self.keys.len() != selectors.len() {
            return false;
        }
        self.keys.iter().zip(selectors).all(|(key, selector)| {
            if key.is_default {
                true
            } else {
                key.key == *selector
            }
        })
    }
}

pub(super) struct ParsedMatchDeclaration<'a> {
    pub(super) selectors: Vec<String>,
    pub(super) arms: Vec<MatchArm<'a>>,
    pub(super) duplicate_keys: Vec<crate::syntax::semantic::VariantKeyOccurrence>,
}

pub(super) fn lower_match_declaration_prelude(
    prelude: crate::syntax::semantic::MatchDeclarationPrelude<'_>,
) -> ParsedMatchDeclaration<'_> {
    let selectors = prelude
        .selectors
        .iter()
        .map(|selector| selector.canonical.clone())
        .collect::<Vec<_>>();

    let mut arms = Vec::with_capacity(prelude.variants.len());
    for variant in &prelude.variants {
        let mut keys = Vec::with_capacity(selectors.len());
        for idx in 0..selectors.len() {
            let key = &variant.keys[idx];
            let is_default = key.is_default;
            keys.push(MatchKey {
                key: key.key.clone(),
                is_default,
                span: key.span.clone(),
                origin: None,
            });
        }

        let pattern = variant.pattern.clone();
        arms.push(MatchArm { keys, pattern });
    }

    ParsedMatchDeclaration {
        selectors,
        arms,
        duplicate_keys: prelude.duplicate_keys,
    }
}

pub(super) fn build_nested_match_ir(
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
    level: usize,
    candidates: &[usize],
    fallback: Option<&[Part]>,
    line: usize,
) -> Result<Vec<Part>, CompileError> {
    if candidates.is_empty() {
        return fallback
            .map(|parts| parts.to_vec())
            .ok_or(CompileError::invalid_expr(line));
    }

    if level >= selectors.len() {
        return Ok(
            arms[*candidates.first().ok_or(CompileError::invalid_expr(line))?]
                .parts
                .clone(),
        );
    }

    if let Some(plan) = numeric_selector_lowering_plan(&selectors[level]) {
        return build_numeric_match_ir(selectors, arms, level, candidates, fallback, line, &plan);
    }

    if level + 1 == selectors.len() {
        return build_leaf_match_ir(selectors, arms, level, candidates, fallback, line);
    }

    let mut seen = BTreeSet::new();
    let mut exact_keys = Vec::new();
    for idx in candidates {
        let key = &arms[*idx].keys[level];
        if !key.is_default && seen.insert(key.key.clone()) {
            exact_keys.push(key.clone());
        }
    }

    let default_candidates = match_candidates(arms, candidates, level, None);
    let default = if default_candidates.is_empty() {
        fallback
            .map(|parts| parts.to_vec())
            .ok_or(CompileError::invalid_expr(line))?
    } else {
        build_nested_match_ir(
            selectors,
            arms,
            level + 1,
            &default_candidates,
            fallback,
            line,
        )?
    };

    if exact_keys.is_empty() {
        return Ok(default);
    }

    let mut lowered_arms = Vec::new();
    for key in exact_keys {
        let branch = exact_match_candidates(arms, candidates, level, &key.key);
        let parts =
            build_nested_match_ir(selectors, arms, level + 1, &branch, Some(&default), line)?;
        lowered_arms.push(select_arm_from_key(key, parts));
    }

    Ok(vec![Part::Select(SelectExpr {
        selector: selectors[level].clone(),
        arms: lowered_arms,
        default,
    })])
}

fn build_numeric_match_ir(
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
    level: usize,
    candidates: &[usize],
    fallback: Option<&[Part]>,
    line: usize,
    plan: &NumericSelectorLoweringPlan,
) -> Result<Vec<Part>, CompileError> {
    let default_candidates = match_candidates(arms, candidates, level, None);
    let default = if default_candidates.is_empty() {
        fallback
            .map(|parts| parts.to_vec())
            .ok_or(CompileError::invalid_expr(line))?
    } else {
        build_nested_match_ir(
            selectors,
            arms,
            level + 1,
            &default_candidates,
            fallback,
            line,
        )?
    };

    let mut seen_exact = BTreeSet::new();
    let mut exact_keys = Vec::new();
    let mut seen_keyword = BTreeSet::new();
    let mut keyword_keys = Vec::new();
    for idx in candidates {
        let key = &arms[*idx].keys[level];
        if key.is_default {
            continue;
        }
        if parse_number_literal(&key.key).is_some() {
            if seen_exact.insert(key.key.clone()) {
                exact_keys.push(key.clone());
            }
        } else if seen_keyword.insert(key.key.clone()) {
            keyword_keys.push(key.clone());
        }
    }

    if exact_keys.is_empty() && keyword_keys.is_empty() {
        return Ok(default);
    }

    let keyword_fallback = if keyword_keys.is_empty() {
        default.clone()
    } else {
        let mut keyword_arms = Vec::with_capacity(keyword_keys.len());
        for key in keyword_keys {
            let branch = exact_match_candidates(arms, candidates, level, &key.key);
            let parts =
                build_nested_match_ir(selectors, arms, level + 1, &branch, Some(&default), line)?;
            keyword_arms.push(select_arm_from_key(key, parts));
        }
        vec![Part::Select(SelectExpr {
            selector: plan.keyword_selector.clone(),
            arms: keyword_arms,
            default: default.clone(),
        })]
    };

    if exact_keys.is_empty() {
        return Ok(keyword_fallback);
    }

    let mut exact_arms = Vec::with_capacity(exact_keys.len());
    for key in exact_keys {
        let branch = exact_match_candidates(arms, candidates, level, &key.key);
        let parts =
            build_nested_match_ir(selectors, arms, level + 1, &branch, Some(&default), line)?;
        exact_arms.push(select_arm_from_key(key, parts));
    }

    Ok(vec![Part::Select(SelectExpr {
        selector: plan.exact_selector.clone(),
        arms: exact_arms,
        default: keyword_fallback,
    })])
}

fn build_leaf_match_ir(
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
    level: usize,
    candidates: &[usize],
    fallback: Option<&[Part]>,
    line: usize,
) -> Result<Vec<Part>, CompileError> {
    let mut first_default = None;
    let mut seen = BTreeSet::new();
    let mut exact_keys = Vec::new();
    let mut first_exact = BTreeMap::new();

    for (position, idx) in candidates.iter().copied().enumerate() {
        let key = &arms[idx].keys[level];
        if key.is_default {
            first_default.get_or_insert((position, idx));
            continue;
        }
        if seen.insert(key.key.clone()) {
            exact_keys.push(key.clone());
            first_exact.insert(key.key.clone(), (position, idx));
        }
    }

    let default = if let Some((_, idx)) = first_default {
        arms[idx].parts.clone()
    } else {
        fallback
            .map(|parts| parts.to_vec())
            .ok_or(CompileError::invalid_expr(line))?
    };
    if exact_keys.is_empty() {
        return Ok(default);
    }

    let mut lowered_arms = Vec::with_capacity(exact_keys.len());
    for key in exact_keys {
        let exact = first_exact
            .get(&key.key)
            .copied()
            .ok_or(CompileError::invalid_expr(line))?;
        lowered_arms.push(select_arm_from_key(key, arms[exact.1].parts.clone()));
    }

    Ok(vec![Part::Select(SelectExpr {
        selector: selectors[level].clone(),
        arms: lowered_arms,
        default,
    })])
}

fn match_candidates(
    arms: &[LoweredMatchArm],
    candidates: &[usize],
    level: usize,
    key: Option<&str>,
) -> Vec<usize> {
    candidates
        .iter()
        .copied()
        .filter(|idx| {
            let arm_key = &arms[*idx].keys[level];
            match key {
                Some(expected) => !arm_key.is_default && arm_key.key == expected,
                None => arm_key.is_default,
            }
        })
        .collect()
}

fn exact_match_candidates(
    arms: &[LoweredMatchArm],
    candidates: &[usize],
    level: usize,
    expected: &str,
) -> Vec<usize> {
    candidates
        .iter()
        .copied()
        .filter(|idx| {
            let arm_key = &arms[*idx].keys[level];
            !arm_key.is_default && arm_key.key == expected
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinNumericSelectorMode {
    Exact,
    Plural,
    Ordinal,
    Dynamic,
}

pub(super) fn builtin_numeric_selector_mode(
    selector: &SelectorExpr,
) -> Option<BuiltinNumericSelectorMode> {
    let SelectorExpr::Call { func, .. } = selector else {
        return None;
    };
    builtin_numeric_selector_mode_for_func(func)
}

pub(super) fn builtin_selector_variant_key_expectation(
    selector: &SelectorExpr,
) -> Option<&'static str> {
    match builtin_numeric_selector_mode(selector) {
        Some(BuiltinNumericSelectorMode::Exact) => Some("MF2 number literal"),
        Some(
            BuiltinNumericSelectorMode::Plural
            | BuiltinNumericSelectorMode::Ordinal
            | BuiltinNumericSelectorMode::Dynamic,
        ) => Some("CLDR plural category or MF2 number literal"),
        None => None,
    }
}

pub(super) fn builtin_selector_accepts_variant_key(selector: &SelectorExpr, key: &str) -> bool {
    match builtin_numeric_selector_mode(selector) {
        Some(BuiltinNumericSelectorMode::Exact) => parse_number_literal(key).is_some(),
        Some(
            BuiltinNumericSelectorMode::Plural
            | BuiltinNumericSelectorMode::Ordinal
            | BuiltinNumericSelectorMode::Dynamic,
        ) => is_cldr_plural_category(key) || parse_number_literal(key).is_some(),
        None => true,
    }
}

fn builtin_numeric_selector_mode_for_func(
    func: &FunctionSpec,
) -> Option<BuiltinNumericSelectorMode> {
    if !matches!(func.name.as_str(), "number" | "integer") {
        return None;
    }

    let mut has_select_option = false;
    let mut select_literal = None;
    for option in &func.options {
        if option.key != "select" {
            continue;
        }
        has_select_option = true;
        match &option.value {
            FunctionOptionValue::Literal(value) => select_literal = Some(value.as_str()),
            FunctionOptionValue::Var(_) => return Some(BuiltinNumericSelectorMode::Dynamic),
        }
    }

    match select_literal {
        Some("exact") => Some(BuiltinNumericSelectorMode::Exact),
        Some("plural") => Some(BuiltinNumericSelectorMode::Plural),
        Some("ordinal") => Some(BuiltinNumericSelectorMode::Ordinal),
        None if !has_select_option => Some(BuiltinNumericSelectorMode::Plural),
        Some(_) => Some(BuiltinNumericSelectorMode::Dynamic),
        None => unreachable!("select option without literal or variable payload"),
    }
}

fn select_arm_from_key(key: MatchKey, parts: Vec<Part>) -> SelectArm {
    SelectArm {
        key: key.key,
        parts,
        origin: key.origin,
    }
}

fn is_cldr_plural_category(key: &str) -> bool {
    matches!(key, "zero" | "one" | "two" | "few" | "many" | "other")
}

#[derive(Clone)]
struct NumericSelectorLoweringPlan {
    exact_selector: SelectorExpr,
    keyword_selector: SelectorExpr,
}

fn numeric_selector_lowering_plan(selector: &SelectorExpr) -> Option<NumericSelectorLoweringPlan> {
    let SelectorExpr::Call { operand, func } = selector else {
        return None;
    };
    match builtin_numeric_selector_mode_for_func(func)? {
        BuiltinNumericSelectorMode::Exact => None,
        BuiltinNumericSelectorMode::Plural => Some(NumericSelectorLoweringPlan {
            exact_selector: clone_numeric_selector_with_select(operand.clone(), func, "exact"),
            keyword_selector: clone_numeric_selector_with_select(operand.clone(), func, "plural"),
        }),
        BuiltinNumericSelectorMode::Ordinal => Some(NumericSelectorLoweringPlan {
            exact_selector: clone_numeric_selector_with_select(operand.clone(), func, "exact"),
            keyword_selector: clone_numeric_selector_with_select(operand.clone(), func, "ordinal"),
        }),
        BuiltinNumericSelectorMode::Dynamic => Some(NumericSelectorLoweringPlan {
            exact_selector: clone_numeric_selector_with_select(operand.clone(), func, "exact"),
            keyword_selector: SelectorExpr::Call {
                operand: operand.clone(),
                func: func.clone(),
            },
        }),
    }
}

fn clone_numeric_selector_with_select(
    operand: Operand,
    func: &FunctionSpec,
    select_value: &str,
) -> SelectorExpr {
    let mut func = func.clone();
    let mut replaced = false;
    for option in &mut func.options {
        if option.key == "select" {
            option.value = FunctionOptionValue::Literal(select_value.to_owned());
            replaced = true;
            break;
        }
    }
    if !replaced {
        func.options
            .push(FunctionOption::literal("select", select_value));
    }
    SelectorExpr::Call { operand, func }
}
