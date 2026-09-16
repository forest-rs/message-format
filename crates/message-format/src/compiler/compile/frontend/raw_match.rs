// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, string::String, vec::Vec};

use super::*;
use crate::compiler::semantic::SelectorExpr;
use crate::compiler::syntax::span::byte_to_line_col;

use super::bindings::DeclarationPlan;
use super::matching::{
    LoweredMatchArm, MatchArm, build_nested_match_ir, builtin_selector_accepts_variant_key,
    builtin_selector_variant_key_expectation, lower_match_declaration_prelude,
};
use super::pattern::{FunctionOriginContext, lower_pattern_node_to_parts};

pub(super) fn lower_raw_match_ir(
    source: &str,
    ctx: SourceContext,
    plan: &DeclarationPlan,
    match_prelude: crate::compiler::syntax::semantic::MatchDeclarationPrelude<'_>,
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
    let arms = lower_match_arm_patterns(
        source,
        parsed_match.arms,
        plan,
        ctx,
        options,
        function_origin,
    )?;
    let selectors = analyze_selectors(source, ctx, &parsed_match.selectors, plan)?;
    validate_match_arms(source, ctx, &selectors, &arms, &parsed_match.duplicate_keys)?;

    let (line, _) = ctx.location(source, 0);
    let candidates = (0..arms.len()).collect::<Vec<_>>();
    let mut dispatch_selectors = selectors;
    let mut checks = Vec::new();
    for selector in &mut dispatch_selectors {
        let SelectorExpr::Local { slot, func } = selector else {
            continue;
        };
        let slot = *slot;
        let func = func.clone();
        if !func
            .as_ref()
            .is_some_and(|func| func.name == "string" && func.options.is_empty())
        {
            checks.push(Part::CheckSelector(slot));
        }
        *selector = SelectorExpr::CheckedLocal { slot, func };
    }
    let mut dispatch =
        build_nested_match_ir(&dispatch_selectors, &arms, 0, &candidates, None, line)?;
    checks.append(&mut dispatch);
    Ok(checks)
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
    plan: &DeclarationPlan,
) -> Result<Vec<SelectorExpr>, CompileError> {
    let mut parts = Vec::with_capacity(selectors.len());
    for selector in selectors {
        parts.push(analyze_selector(source, ctx, selector, plan)?);
    }
    Ok(parts)
}

fn analyze_selector(
    source: &str,
    ctx: SourceContext,
    selector: &str,
    plan: &DeclarationPlan,
) -> Result<SelectorExpr, CompileError> {
    let name = plan.resolved_name(selector);
    let Some(function) = plan.function(selector) else {
        let (line, col) = ctx.location(source, 0);
        return Err(CompileError::missing_selector_annotation_detail(
            line,
            col,
            "selector with function annotation",
            format!("selector ${name} has no function annotation"),
        ));
    };
    let Some(slot) = plan.slot(selector) else {
        return Err(CompileError::internal("annotated selector without storage"));
    };
    Ok(SelectorExpr::Local {
        slot,
        func: Some(function.func.clone()),
    })
}

fn validate_match_arms(
    source: &str,
    ctx: SourceContext,
    selectors: &[SelectorExpr],
    arms: &[LoweredMatchArm],
    duplicate_keys: &[crate::compiler::syntax::semantic::VariantKeyOccurrence],
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
        crate::compiler::syntax::semantic::validate_no_duplicate_single_selector_keys(
            duplicate_keys,
        )?;
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

fn lower_match_arm_patterns(
    source: &str,
    arms: Vec<MatchArm<'_>>,
    plan: &DeclarationPlan,
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
        let parts = lower_pattern_node_to_parts(
            source,
            &arm.pattern,
            arm_ctx,
            options,
            function_origin,
            Some(plan),
        )?;
        out.push(LoweredMatchArm {
            keys: arm.keys,
            parts,
        });
    }
    Ok(out)
}
