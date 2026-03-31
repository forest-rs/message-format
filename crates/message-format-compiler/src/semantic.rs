// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal compiler semantic model used between frontend analysis and lowering.
//!
//! This module is intentionally compiler-shaped. It is the compiler's current
//! internal meaning/lowering model, not a stable resource authoring schema.
//! External producers may use it when they already have structured message
//! data, but should treat it as a low-level compiler input boundary rather than
//! a long-term calm public IR.

/// Parsed catalog message definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Stable message id.
    pub id: String,
    /// Parsed template parts.
    pub parts: Vec<Part>,
    /// Original MF2 message body text, when this message came from MF2 source.
    ///
    /// This is retained for nested diagnostic rendering. Structured builders
    /// may leave it unset.
    pub message_source: Option<String>,
    /// Optional container/input provenance for this message.
    pub origin: Option<SourceSpan>,
    /// Optional upstream/original provenance supplied by a resource adapter.
    pub upstream_origin: Option<DetachedSourceSpan>,
}

impl Message {
    /// Start building one structured semantic message.
    ///
    /// This is the lowest-level public entry point for non-text producers that
    /// want to compile directly into a catalog without going through `.mf2`
    /// text.
    ///
    /// It builds the compiler's current internal semantic model. Prefer
    /// resource adapters or higher-level container APIs when you have a real
    /// resource format rather than compiler-shaped message parts.
    #[must_use]
    pub fn builder(id: impl Into<String>) -> MessageBuilder {
        MessageBuilder::new(id)
    }
}

/// Builder for one compiler-semantic [`Message`].
///
/// This is a fluent constructor for the compiler's current internal semantic
/// model. It is useful for generated or already-structured inputs, but it is
/// not intended to define a stable end-user message authoring format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBuilder {
    id: String,
    parts: Vec<Part>,
    message_source: Option<String>,
    origin: Option<SourceSpan>,
    upstream_origin: Option<DetachedSourceSpan>,
}

impl MessageBuilder {
    /// Create a new builder for one message id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parts: Vec::new(),
            message_source: None,
            origin: None,
            upstream_origin: None,
        }
    }

    /// Attach original MF2 message body text for diagnostic rendering.
    #[must_use]
    pub fn message_source(mut self, source: impl Into<String>) -> Self {
        self.message_source = Some(source.into());
        self
    }

    /// Attach message-level provenance.
    #[must_use]
    pub fn origin(mut self, origin: SourceSpan) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Attach upstream/original provenance supplied by a resource container.
    #[must_use]
    pub fn upstream_origin(mut self, origin: DetachedSourceSpan) -> Self {
        self.upstream_origin = Some(origin);
        self
    }

    /// Push one prebuilt semantic part.
    #[must_use]
    pub fn part(mut self, part: Part) -> Self {
        self.parts.push(part);
        self
    }

    /// Push one text segment.
    #[must_use]
    pub fn text(self, value: impl Into<String>) -> Self {
        self.part(Part::text(value))
    }

    /// Push one literal expression segment.
    #[must_use]
    pub fn literal(self, value: impl Into<String>) -> Self {
        self.part(Part::literal(value))
    }

    /// Push one variable interpolation segment.
    #[must_use]
    pub fn var(self, name: impl Into<String>) -> Self {
        self.part(Part::var(name))
    }

    /// Push one call part.
    #[must_use]
    pub fn call(self, call: CallExpr) -> Self {
        self.part(Part::call(call))
    }

    /// Push one select part.
    #[must_use]
    pub fn select(self, select: SelectExpr) -> Self {
        self.part(Part::select(select))
    }

    /// Push one markup-open part.
    #[must_use]
    pub fn markup_open(
        self,
        name: impl Into<String>,
        options: impl Into<Vec<FunctionOption>>,
    ) -> Self {
        self.part(Part::markup_open(name, options))
    }

    /// Push one markup-close part.
    #[must_use]
    pub fn markup_close(
        self,
        name: impl Into<String>,
        options: impl Into<Vec<FunctionOption>>,
    ) -> Self {
        self.part(Part::markup_close(name, options))
    }

    /// Finish the message.
    #[must_use]
    pub fn build(self) -> Message {
        Message {
            id: self.id,
            parts: self.parts,
            message_source: self.message_source,
            origin: self.origin,
            upstream_origin: self.upstream_origin,
        }
    }
}

/// One compiler-semantic message part.
///
/// [`Part`] deliberately mixes the body forms the compiler currently needs to
/// lower: text, interpolation, calls, selects, and markup. It is an internal
/// compiler model, not a polished long-term HIR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// Literal text bytes copied to literal blob.
    Text(String),
    /// Literal expression value, e.g. `{foo}` or `{|foo|}`.
    Literal(String),
    /// Variable interpolation from caller args.
    Var(String),
    /// Function call on one operand.
    Call(CallExpr),
    /// Select expression with case arms and a required default arm.
    Select(SelectExpr),
    /// Markup open tag, e.g. `{#bold}` or `{#link href=$url}`.
    MarkupOpen {
        /// Tag name.
        name: String,
        /// Markup options (key-value pairs).
        options: Vec<FunctionOption>,
    },
    /// Markup close tag, e.g. `{/bold}`.
    MarkupClose {
        /// Tag name.
        name: String,
        /// Markup options (key-value pairs).
        options: Vec<FunctionOption>,
    },
}

impl Part {
    /// Construct one text part.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct one literal-expression part.
    #[must_use]
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }

    /// Construct one variable interpolation part.
    #[must_use]
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var(name.into())
    }

    /// Construct one function-call part.
    #[must_use]
    pub fn call(call: CallExpr) -> Self {
        Self::Call(call)
    }

    /// Construct one select part.
    #[must_use]
    pub fn select(select: SelectExpr) -> Self {
        Self::Select(select)
    }

    /// Construct one markup-open part.
    #[must_use]
    pub fn markup_open(name: impl Into<String>, options: impl Into<Vec<FunctionOption>>) -> Self {
        Self::MarkupOpen {
            name: name.into(),
            options: options.into(),
        }
    }

    /// Construct one markup-close part.
    #[must_use]
    pub fn markup_close(name: impl Into<String>, options: impl Into<Vec<FunctionOption>>) -> Self {
        Self::MarkupClose {
            name: name.into(),
            options: options.into(),
        }
    }
}

/// Selector expression used by `.match` lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorExpr {
    /// Variable selector.
    Var(String),
    /// Function call selector.
    Call {
        /// Selector operand.
        operand: Operand,
        /// Function specification.
        func: FunctionSpec,
    },
    /// Literal selector expression.
    Literal(String),
}

impl SelectorExpr {
    /// Construct one variable selector.
    #[must_use]
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var(name.into())
    }

    /// Construct one literal selector.
    #[must_use]
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }

    /// Construct one function-call selector.
    #[must_use]
    pub fn call(operand: Operand, func: FunctionSpec) -> Self {
        Self::Call { operand, func }
    }
}

/// Operand used by call and selector expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// Variable operand.
    Var(String),
    /// Literal operand.
    Literal {
        /// Decoded literal text.
        value: String,
        /// Source-level literal classification.
        kind: OperandLiteralKind,
    },
}

/// Literal classification for function/select operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandLiteralKind {
    /// A quoted/string-like literal operand.
    String,
    /// A numeric literal operand.
    Number,
}

impl Operand {
    /// Construct one variable operand.
    #[must_use]
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var(name.into())
    }

    /// Construct one literal operand.
    #[must_use]
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal {
            value: value.into(),
            kind: OperandLiteralKind::String,
        }
    }

    /// Construct one numeric literal operand.
    #[must_use]
    pub fn number_literal(value: impl Into<String>) -> Self {
        Self::Literal {
            value: value.into(),
            kind: OperandLiteralKind::Number,
        }
    }

    /// Returns the operand text when this is a literal.
    #[must_use]
    pub fn literal_value(&self) -> Option<&str> {
        match self {
            Self::Literal { value, .. } => Some(value.as_str()),
            Self::Var(_) => None,
        }
    }

    /// Returns the literal classification when this is a literal.
    #[must_use]
    pub fn literal_kind(&self) -> Option<OperandLiteralKind> {
        match self {
            Self::Literal { kind, .. } => Some(*kind),
            Self::Var(_) => None,
        }
    }
}

/// Function call expression in the semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallExpr {
    /// Call operand.
    pub operand: Operand,
    /// Function specification.
    pub func: FunctionSpec,
    /// Explicit fallback string for error recovery (set when inlining locals).
    pub fallback: Option<String>,
}

impl CallExpr {
    /// Construct one call expression with no explicit fallback.
    #[must_use]
    pub fn new(operand: Operand, func: FunctionSpec) -> Self {
        Self {
            operand,
            func,
            fallback: None,
        }
    }

    /// Attach an explicit fallback string for error recovery.
    #[must_use]
    pub fn fallback(mut self, fallback: impl Into<String>) -> Self {
        self.fallback = Some(fallback.into());
        self
    }
}

/// Select expression in the semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectExpr {
    /// Selector expression.
    pub selector: SelectorExpr,
    /// Non-default case arms.
    pub arms: Vec<SelectArm>,
    /// Default arm parts.
    pub default: Vec<Part>,
}

impl SelectExpr {
    /// Start building one select expression.
    #[must_use]
    pub fn builder(selector: SelectorExpr) -> SelectExprBuilder {
        SelectExprBuilder::new(selector)
    }
}

/// Builder for one semantic [`SelectExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectExprBuilder {
    selector: SelectorExpr,
    arms: Vec<SelectArm>,
    default: Vec<Part>,
}

impl SelectExprBuilder {
    /// Create a new builder for one selector expression.
    #[must_use]
    pub fn new(selector: SelectorExpr) -> Self {
        Self {
            selector,
            arms: Vec::new(),
            default: Vec::new(),
        }
    }

    /// Add one non-default match arm.
    #[must_use]
    pub fn arm(mut self, key: impl Into<String>, parts: impl Into<Vec<Part>>) -> Self {
        self.arms.push(SelectArm::new(key, parts));
        self
    }

    /// Set the default arm parts.
    #[must_use]
    pub fn default(mut self, parts: impl Into<Vec<Part>>) -> Self {
        self.default = parts.into();
        self
    }

    /// Finish the select expression.
    #[must_use]
    pub fn build(self) -> SelectExpr {
        SelectExpr {
            selector: self.selector,
            arms: self.arms,
            default: self.default,
        }
    }
}

/// Function name + options carried by call parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSpec {
    /// Function identifier.
    pub name: String,
    /// Parsed options (static + dynamic).
    pub options: Vec<FunctionOption>,
    /// Optional provenance for this function annotation or generated function.
    pub origin: Option<SourceSpan>,
}

impl FunctionSpec {
    /// Construct one function specification with no options or provenance.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            options: Vec::new(),
            origin: None,
        }
    }

    /// Push one prebuilt option.
    #[must_use]
    pub fn option(mut self, option: FunctionOption) -> Self {
        self.options.push(option);
        self
    }

    /// Push one literal-valued option.
    #[must_use]
    pub fn option_literal(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.option(FunctionOption::literal(key, value))
    }

    /// Push one variable-valued option.
    #[must_use]
    pub fn option_var(self, key: impl Into<String>, name: impl Into<String>) -> Self {
        self.option(FunctionOption::var(key, name))
    }

    /// Attach function-level provenance.
    #[must_use]
    pub fn origin(mut self, origin: SourceSpan) -> Self {
        self.origin = Some(origin);
        self
    }
}

/// One function option token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionOption {
    /// Option key.
    pub key: String,
    /// Option value.
    pub value: FunctionOptionValue,
}

impl FunctionOption {
    /// Construct one literal-valued option.
    #[must_use]
    pub fn literal(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: FunctionOptionValue::Literal(value.into()),
        }
    }

    /// Construct one variable-valued option.
    #[must_use]
    pub fn var(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: FunctionOptionValue::Var(name.into()),
        }
    }
}

/// Option value kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionOptionValue {
    /// Static literal value.
    Literal(String),
    /// Dynamic variable option value.
    Var(String),
}

/// One select case arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectArm {
    /// Case match key.
    pub key: String,
    /// Lowered arm parts.
    pub parts: Vec<Part>,
    /// Optional provenance for the authored variant key.
    pub origin: Option<SourceSpan>,
}

impl SelectArm {
    /// Construct one select arm.
    #[must_use]
    pub fn new(key: impl Into<String>, parts: impl Into<Vec<Part>>) -> Self {
        Self {
            key: key.into(),
            parts: parts.into(),
            origin: None,
        }
    }

    /// Attach variant-key provenance.
    #[must_use]
    pub fn origin(mut self, origin: SourceSpan) -> Self {
        self.origin = Some(origin);
        self
    }
}

/// Stable source-table id for compiler provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u32);

/// Logical kind of source material contributing to a compiled catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// `MessageFormat` source text.
    MessageFormat,
    /// Generated source not tied to a language-specific input file.
    Generated,
    /// Rust or macro-generated source.
    Rust,
    /// XLIFF interchange content.
    Xliff,
    /// Another structured source or project-specific source kind.
    Other(String),
}

/// One registered source in a compiler source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// Stable source id used by spans.
    pub id: SourceId,
    /// Human-facing display name or logical path.
    pub name: String,
    /// Logical source kind.
    pub kind: SourceKind,
}

/// Optional source provenance for one semantic message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// Source-table entry containing this span.
    pub source_id: SourceId,
    /// Start byte offset in the source.
    pub byte_start: usize,
    /// End byte offset in the source.
    pub byte_end: usize,
    /// Optional 1-based line number.
    pub line: Option<usize>,
    /// Optional 1-based column number.
    pub column: Option<usize>,
}

/// Optional upstream/original provenance not tied to the compiler source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedSourceSpan {
    /// Human-facing source label or path.
    pub name: String,
    /// Logical source kind.
    pub kind: SourceKind,
    /// Optional start byte offset in the upstream source.
    pub byte_start: Option<usize>,
    /// Optional end byte offset in the upstream source.
    pub byte_end: Option<usize>,
    /// Optional 1-based line number.
    pub line: Option<usize>,
    /// Optional 1-based column number.
    pub column: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_builder_constructs_structured_message() {
        let message = Message::builder("main")
            .text("Hello ")
            .var("name")
            .text("!")
            .build();

        assert_eq!(
            message,
            Message {
                id: String::from("main"),
                parts: vec![
                    Part::Text(String::from("Hello ")),
                    Part::Var(String::from("name")),
                    Part::Text(String::from("!")),
                ],
                message_source: None,
                origin: None,
                upstream_origin: None,
            }
        );
    }

    #[test]
    fn message_builder_attaches_upstream_origin() {
        let message = Message::builder("main")
            .upstream_origin(DetachedSourceSpan {
                name: String::from("src/app.rs"),
                kind: SourceKind::Rust,
                byte_start: Some(10),
                byte_end: Some(20),
                line: Some(3),
                column: Some(7),
            })
            .build();

        assert_eq!(
            message.upstream_origin,
            Some(DetachedSourceSpan {
                name: String::from("src/app.rs"),
                kind: SourceKind::Rust,
                byte_start: Some(10),
                byte_end: Some(20),
                line: Some(3),
                column: Some(7),
            })
        );
    }

    #[test]
    fn select_expr_builder_constructs_select() {
        let select = SelectExpr::builder(SelectorExpr::var("kind"))
            .arm("formal", vec![Part::text("Good evening")])
            .default(vec![Part::text("Hi")])
            .build();

        assert_eq!(
            select,
            SelectExpr {
                selector: SelectorExpr::Var(String::from("kind")),
                arms: vec![SelectArm {
                    key: String::from("formal"),
                    parts: vec![Part::Text(String::from("Good evening"))],
                    origin: None,
                }],
                default: vec![Part::Text(String::from("Hi"))],
            }
        );
    }
}
