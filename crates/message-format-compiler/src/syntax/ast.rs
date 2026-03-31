// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Syntax tree scaffolding for TR35-aligned parser migration.

use core::ops::Range;

/// Parsed message document with declaration prelude and remaining pattern body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxDocument<'a> {
    pub(crate) declarations: Vec<DeclarationNode<'a>>,
    pub(crate) body: Option<PatternNode<'a>>,
}

/// Parsed declaration node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclarationNode<'a> {
    pub(crate) kind: DeclarationKind,
    pub(crate) span: Range<usize>,
    pub(crate) payload: Option<DeclarationPayloadNode<'a>>,
}

/// Declaration head kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclarationKind {
    Input,
    Local,
    Match,
}

/// Parsed declaration payload details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeclarationPayloadNode<'a> {
    Input {
        expr: ExpressionNode<'a>,
    },
    Local {
        name_span: Range<usize>,
        name: &'a str,
        expr: ExpressionNode<'a>,
    },
    Match {
        selectors: Vec<ExpressionNode<'a>>,
        variants: Vec<MatchVariantNode<'a>>,
    },
}

/// Parsed `.match` variant payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchVariantNode<'a> {
    pub(crate) keys: Vec<VariantKeyNode<'a>>,
    pub(crate) pattern: PatternNode<'a>,
}

/// Parsed `.match` variant key payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VariantKeyNode<'a> {
    pub(crate) span: Range<usize>,
    pub(crate) text: &'a str,
    pub(crate) is_default: bool,
    pub(crate) is_quoted: bool,
}

/// Remaining pattern body after declaration prelude parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatternNode<'a> {
    pub(crate) span: Range<usize>,
    pub(crate) segments: Vec<PatternSegmentNode<'a>>,
}

/// Parsed pattern segment (raw text or expression).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PatternSegmentNode<'a> {
    Text { span: Range<usize>, value: &'a str },
    Expression(Box<ExpressionNode<'a>>),
}

/// Parsed expression payload inside a pattern body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpressionNode<'a> {
    pub(crate) raw_span: Range<usize>,
    pub(crate) span: Range<usize>,
    pub(crate) kind: ExpressionKindNode,
    pub(crate) payload: Option<ExpressionPayloadNode<'a>>,
    pub(crate) diag_hint: Option<ExpressionDiagnosticHint>,
}

/// Parsed expression shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpressionKindNode {
    Var,
    Call,
    Literal,
}

/// Parser-provided diagnostic hints for malformed expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpressionDiagnosticHint {
    NonSelectPayloadUnavailable,
    FunctionNameRequiredAfterColon,
}

/// Parsed non-select expression payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExpressionPayloadNode<'a> {
    Var(VarExpressionNode<'a>),
    Call(CallExpressionNode<'a>),
    Literal(LiteralExpressionNode<'a>),
    Markup(MarkupNode<'a>),
}

/// Parsed variable expression payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VarExpressionNode<'a> {
    pub(crate) name_span: Range<usize>,
    pub(crate) name: &'a str,
}

/// Parsed call expression payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallExpressionNode<'a> {
    pub(crate) operand: CallOperandNode<'a>,
    pub(crate) function: FunctionSpecNode<'a>,
}

/// Parsed call operand payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallOperandNode<'a> {
    Var(VarExpressionNode<'a>),
    #[allow(
        dead_code,
        reason = "Literal operands are represented for semantic checks and future lowering paths."
    )]
    Literal {
        value_span: Range<usize>,
        value: &'a str,
    },
}

/// Parsed literal expression payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiteralExpressionNode<'a> {
    pub(crate) value_span: Range<usize>,
    pub(crate) value: &'a str,
    pub(crate) function: Option<FunctionSpecNode<'a>>,
    pub(crate) is_markup: bool,
}

/// Parsed function name and option token spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionSpecNode<'a> {
    pub(crate) name_span: Range<usize>,
    pub(crate) name: &'a str,
    pub(crate) option_spans: Vec<Range<usize>>,
    pub(crate) options: Vec<&'a str>,
    pub(crate) structured_options: Vec<OptionNode<'a>>,
    pub(crate) attributes: Vec<AttributeNode<'a>>,
}

/// A parsed option: `key = value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionNode<'a> {
    pub(crate) name: &'a str,
    pub(crate) value: OptionValue<'a>,
    pub(crate) span: Range<usize>,
}

/// Option or attribute value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OptionValue<'a> {
    Literal(&'a str),
    Variable(&'a str),
}

/// A parsed attribute: `@key` or `@key = value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttributeNode<'a> {
    pub(crate) name: &'a str,
    pub(crate) value: Option<OptionValue<'a>>,
    pub(crate) span: Range<usize>,
}

/// Markup expression: `{#tag ...}`, `{/tag ...}`, `{#tag ... /}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkupNode<'a> {
    pub(crate) kind: MarkupKind,
    pub(crate) identifier: &'a str,
    pub(crate) options: Vec<OptionNode<'a>>,
    pub(crate) attributes: Vec<AttributeNode<'a>>,
    pub(crate) span: Range<usize>,
}

/// Markup kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkupKind {
    Open,
    Close,
    SelfClose,
}
