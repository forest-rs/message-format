// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::semantic::SelectorExpr;
use crate::syntax::span::byte_to_line_col;

use super::bindings::{DeclFunction, DeclarationBindings, LocalValue};
use super::local_eval::{resolve_alias, selector_chain_is_unstable};
use super::matching::{
    LoweredMatchArm, MatchArm, build_nested_match_ir, builtin_selector_accepts_variant_key,
    builtin_selector_variant_key_expectation, lower_match_declaration_prelude,
};
use super::pattern::{FunctionOriginContext, lower_pattern_node_to_parts};
use super::rewrite::lower_parts_with_declaration_bindings;

struct AnalyzedSelectors {
    parts: Vec<SelectorExpr>,
    compile_time_values: Option<Vec<String>>,
    has_unstable_chain: bool,
}

pub(super) fn lower_raw_match_ir(
    source: &str,
    ctx: SourceContext,
    bindings: &DeclarationBindings,
    match_prelude: crate::syntax::semantic::MatchDeclarationPrelude<'_>,
    options: CompileOptions,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Vec<Part>, CompileError> {
    let mut parsed_match = lower_match_declaration_prelude(match_prelude);
    annotate_match_key_origins(
        source,
        ctx,
        function_origin.map(|origin| origin.source_id),
        &mut parsed_match.arms,
    );
    let selectors = analyze_selectors(source, ctx, &parsed_match.selectors, bindings)?;

    let arms = lower_match_arm_patterns(
        source,
        parsed_match.arms,
        bindings,
        ctx,
        options,
        function_origin,
    )?;
    validate_match_arms(
        source,
        ctx,
        &selectors.parts,
        &arms,
        &parsed_match.duplicate_keys,
    )?;

    if selectors.has_unstable_chain {
        return default_arm_parts(source, ctx, &arms);
    }

    if let Some(selector_values) = selectors.compile_time_values.as_ref() {
        return resolve_compile_time_arm(source, ctx, selector_values, &arms);
    }

    let (line, _) = ctx.location(source, 0);
    let candidates = (0..arms.len()).collect::<Vec<_>>();
    build_nested_match_ir(&selectors.parts, &arms, 0, &candidates, None, line)
}

fn annotate_match_key_origins(
    source: &str,
    ctx: SourceContext,
    source_id: Option<SourceId>,
    arms: &mut [MatchArm<'_>],
) {
    let Some(source_id) = source_id else {
        return;
    };
    for arm in arms {
        for key in &mut arm.keys {
            let (line, column) = ctx.location(source, key.span.start);
            key.origin = Some(SourceSpan {
                source_id,
                byte_start: key.span.start,
                byte_end: key.span.end,
                line: Some(line),
                column: Some(column),
            });
        }
    }
}

fn analyze_selectors(
    source: &str,
    ctx: SourceContext,
    selectors: &[String],
    bindings: &DeclarationBindings,
) -> Result<AnalyzedSelectors, CompileError> {
    let mut parts = Vec::with_capacity(selectors.len());
    let mut compile_time_values = Vec::with_capacity(selectors.len());
    let mut all_compile_time = true;
    let mut has_unstable_chain = false;

    for selector in selectors {
        let analyzed = analyze_selector(source, ctx, selector, bindings)?;
        has_unstable_chain |= analyzed.is_unstable;
        all_compile_time &= analyzed.compile_time_value.is_some();
        compile_time_values.push(analyzed.compile_time_value.unwrap_or_default());
        parts.push(analyzed.part);
    }

    Ok(AnalyzedSelectors {
        parts,
        compile_time_values: all_compile_time.then_some(compile_time_values),
        has_unstable_chain,
    })
}

struct AnalyzedSelector {
    part: SelectorExpr,
    compile_time_value: Option<String>,
    is_unstable: bool,
}

fn analyze_selector(
    source: &str,
    ctx: SourceContext,
    selector: &str,
    bindings: &DeclarationBindings,
) -> Result<AnalyzedSelector, CompileError> {
    let name = resolve_alias(selector, &bindings.aliases)?;
    let literal = bindings
        .locals
        .get(&name)
        .and_then(LocalValue::as_literal)
        .map(ToOwned::to_owned);
    let part = bindings
        .local_functions
        .get(&name)
        .cloned()
        .or_else(|| bindings.input_functions.get(&name).cloned())
        .map(selector_expr_from_decl_function)
        .unwrap_or_else(|| SelectorExpr::Var(name.clone()));
    if matches!(part, SelectorExpr::Var(_)) {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::missing_selector_annotation_detail(
            line,
            col,
            "selector with function annotation",
            format!("selector ${name} has no function annotation"),
        ));
    }
    Ok(AnalyzedSelector {
        compile_time_value: literal.filter(|value| value.is_ascii()),
        is_unstable: selector_chain_is_unstable(&name, &bindings.local_functions),
        part,
    })
}

fn selector_expr_from_decl_function(function: DeclFunction) -> SelectorExpr {
    match function.operand {
        Operand::Var(var) => SelectorExpr::Call {
            operand: Operand::Var(var),
            func: function.func,
        },
        Operand::Literal { value, kind } => SelectorExpr::Call {
            operand: Operand::Literal { value, kind },
            func: function.func,
        },
    }
}

fn validate_match_arms(
    source: &str,
    ctx: SourceContext,
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
    duplicate_keys: &[crate::syntax::semantic::VariantKeyOccurrence],
) -> Result<(), CompileError> {
    if arms.is_empty() {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::invalid_expr_detail(
            line,
            col,
            "at least one match arm",
            "end of input",
        ));
    }

    let default_count = arms.iter().filter(|arm| arm.is_default()).count();
    if default_count == 0 {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::missing_default_arm_detail(
            line,
            col,
            "default arm '*'",
            "no default arm",
        ));
    }
    if default_count > 1 {
        let (line, _) = ctx.location(source, 0);
        return Err(CompileError::duplicate_variant(line));
    }
    if selectors.len() == 1 {
        crate::syntax::semantic::validate_no_duplicate_single_selector_keys(duplicate_keys)?;
    }
    validate_builtin_selector_variant_keys(source, ctx, selectors, arms)?;

    Ok(())
}

fn validate_builtin_selector_variant_keys(
    source: &str,
    ctx: SourceContext,
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
) -> Result<(), CompileError> {
    for (selector_index, selector) in selectors.iter().enumerate() {
        let Some(expected) = builtin_selector_variant_key_expectation(selector) else {
            continue;
        };
        for arm in arms {
            let key = &arm.keys[selector_index];
            if key.is_default || builtin_selector_accepts_variant_key(selector, &key.key) {
                continue;
            }
            let (line, column) = ctx.location(source, key.span.start);
            return Err(CompileError::invalid_variant_key_detail(
                line,
                column,
                expected,
                key.key.clone(),
            ));
        }
    }
    Ok(())
}

fn default_arm_parts(
    source: &str,
    ctx: SourceContext,
    arms: &[LoweredMatchArm],
) -> Result<Vec<Part>, CompileError> {
    arms.iter()
        .find(|arm| arm.is_default())
        .map(|arm| arm.parts.clone())
        .ok_or_else(|| {
            let (line, _) = ctx.location(source, 0);
            CompileError::missing_default_arm(line)
        })
}

fn resolve_compile_time_arm(
    source: &str,
    ctx: SourceContext,
    selector_values: &[String],
    arms: &[LoweredMatchArm],
) -> Result<Vec<Part>, CompileError> {
    if let Some(arm) = arms.iter().find(|arm| arm.matches(selector_values)) {
        return Ok(arm.parts.clone());
    }
    if let Some(default_arm) = arms.iter().find(|arm| arm.is_default()) {
        return Ok(default_arm.parts.clone());
    }
    let (line, _) = ctx.location(source, 0);
    Err(CompileError::missing_default_arm(line))
}

fn lower_match_arm_patterns(
    source: &str,
    arms: Vec<MatchArm<'_>>,
    bindings: &DeclarationBindings,
    ctx: SourceContext,
    options: CompileOptions,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Vec<LoweredMatchArm>, CompileError> {
    let mut out = Vec::with_capacity(arms.len());
    for arm in arms {
        let arm_ctx = match ctx {
            SourceContext::Document => {
                let pattern_byte_start = arm.pattern.span.start;
                let (line, col) = byte_to_line_col(source, pattern_byte_start);
                SourceContext::Line {
                    line,
                    column_offset: col.saturating_sub(1),
                }
            }
            other => other,
        };
        let mut parts =
            lower_pattern_node_to_parts(source, &arm.pattern, arm_ctx, options, function_origin)?;
        lower_parts_with_declaration_bindings(&mut parts, bindings, false)?;
        out.push(LoweredMatchArm {
            keys: arm.keys,
            parts,
        });
    }
    Ok(out)
}
