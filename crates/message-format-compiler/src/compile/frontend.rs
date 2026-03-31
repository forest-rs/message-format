// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::syntax::span::SourceContext;

mod bindings;
mod document;
mod local_eval;
mod matching;
mod pattern;
mod raw_match;
mod rewrite;

use document::parse_single_message;
use pattern::FunctionOriginContext;

#[cfg(test)]
pub(super) fn parse_messages(
    source: &str,
    options: CompileOptions,
) -> Result<Vec<Message>, CompileError> {
    Ok(vec![parse_single_message(source, options, None)?])
}

pub(super) fn parse_single_message_with_source(
    source: &str,
    options: CompileOptions,
    source_id: Option<SourceId>,
) -> Result<Message, CompileError> {
    parse_single_message(source, options, source_id)
}

pub(super) fn lower_expression_node_to_part(
    source: &str,
    expr: &crate::syntax::ast::ExpressionNode<'_>,
    ctx: SourceContext,
    function_origin: Option<FunctionOriginContext>,
) -> Result<Part, CompileError> {
    pattern::lower_expression_node_to_part(source, expr, ctx, function_origin)
}
