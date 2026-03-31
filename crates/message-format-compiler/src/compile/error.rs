// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt;

use crate::manifest::{FunctionOperandKind, FunctionOptionValueKind};
use crate::semantic::SourceSpan;

/// Compiler errors for parse and lowering failures.
#[derive(Debug)]
pub enum CompileError {
    /// Input line is malformed.
    InvalidLine {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Message id is empty.
    EmptyMessageId {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Template expression is malformed.
    InvalidExpr {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Variable name is malformed.
    InvalidVar {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Select expression has no default arm.
    MissingDefaultArm {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Duplicate `.input` or `.local` declaration for the same variable.
    DuplicateDeclaration {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Duplicate variant key (single-selector) or key tuple (multi-selector).
    DuplicateVariant {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Duplicate option name in a function call.
    DuplicateOptionName {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Duplicate message id across one multi-source build.
    DuplicateMessageId {
        /// Repeated message id.
        message_id: String,
        /// First origin where this id was seen.
        first_origin: Option<Box<SourceSpan>>,
        /// Second origin where this id was seen.
        second_origin: Option<Box<SourceSpan>>,
    },
    /// Function name is not present in the supplied function manifest.
    UnknownFunction {
        /// Function identifier.
        function: String,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Function is used in a disallowed call site according to the supplied manifest.
    UnsupportedFunctionUsage {
        /// Function identifier.
        function: String,
        /// Usage site, e.g. `format` or `select`.
        usage: &'static str,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Function option key is not present in the supplied manifest.
    UnknownFunctionOption {
        /// Function identifier.
        function: String,
        /// Option key.
        option: String,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Function option value shape does not match the supplied manifest.
    InvalidFunctionOptionValue {
        /// Function identifier.
        function: String,
        /// Option key.
        option: String,
        /// Expected value shape.
        expected: FunctionOptionValueKind,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Builtin option literal is not one of the accepted values for that builtin.
    InvalidBuiltinOptionValue {
        /// Builtin function identifier.
        function: String,
        /// Option key.
        option: String,
        /// Human-readable accepted value set.
        expected: &'static str,
        /// Human-readable observed literal value.
        found: String,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// One required function option is missing from the supplied call.
    MissingFunctionOption {
        /// Function identifier.
        function: String,
        /// Missing option key.
        option: String,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Literal operand does not match the manifest's declared operand kind.
    InvalidFunctionOperand {
        /// Function identifier.
        function: String,
        /// Expected literal operand kind.
        expected: FunctionOperandKind,
        /// Human-readable observed operand.
        found: String,
        /// Message being validated, if known.
        message_id: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// Selector variable lacks a function annotation.
    MissingSelectorAnnotation {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Variant key count does not match selector count.
    VariantKeyMismatch {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable parser expectation.
        expected: Option<&'static str>,
        /// Human-readable parser observation.
        found: Option<String>,
    },
    /// Variant key is not valid for the selector function.
    InvalidVariantKey {
        /// 1-based line number.
        line: usize,
        /// 1-based column number.
        column: usize,
        /// Human-readable accepted key domain.
        expected: Option<&'static str>,
        /// Human-readable observed key.
        found: Option<String>,
        /// Message origin, if known.
        origin: Option<Box<SourceSpan>>,
    },
    /// File I/O or encoding error from `message_format::compile_file`.
    IoError {
        /// Path that failed to load.
        path: std::path::PathBuf,
        /// Underlying I/O failure.
        source: std::io::Error,
    },
    /// Function id exceeded u16 range.
    FunctionIdOverflow,
    /// String interning exceeded supported id range.
    TooManyStrings,
    /// A non-string compiler structure exceeded its supported size range.
    SizeOverflow {
        /// Human-readable name of the overflowing structure.
        what: &'static str,
    },
    /// A resource/container adapter produced invalid input for compilation.
    ResourceInputError {
        /// Human-readable adapter failure detail.
        detail: String,
    },
    /// Alias resolution exceeded the compiler's supported chain depth.
    AliasResolutionOverflow {
        /// Alias name that could not be resolved safely.
        alias: String,
    },
    /// Internal compiler consistency error (should not be reachable from valid frontend output).
    InternalError {
        /// Description of the consistency failure.
        detail: String,
    },
}

impl CompileError {
    pub(crate) fn internal(detail: impl Into<String>) -> Self {
        Self::InternalError {
            detail: detail.into(),
        }
    }

    pub(crate) fn size_overflow(what: &'static str) -> Self {
        Self::SizeOverflow { what }
    }

    pub(crate) fn alias_resolution_overflow(alias: impl Into<String>) -> Self {
        Self::AliasResolutionOverflow {
            alias: alias.into(),
        }
    }

    pub(crate) fn invalid_expr(line: usize) -> Self {
        Self::InvalidExpr {
            line,
            column: 1,
            expected: None,
            found: None,
        }
    }

    pub(crate) fn invalid_expr_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::InvalidExpr {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn invalid_var_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::InvalidVar {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn missing_default_arm(line: usize) -> Self {
        Self::MissingDefaultArm {
            line,
            column: 1,
            expected: None,
            found: None,
        }
    }

    pub(crate) fn missing_default_arm_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::MissingDefaultArm {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn duplicate_declaration_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::DuplicateDeclaration {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn duplicate_variant(line: usize) -> Self {
        Self::DuplicateVariant {
            line,
            column: 1,
            expected: None,
            found: None,
        }
    }

    pub(crate) fn duplicate_variant_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::DuplicateVariant {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn duplicate_option_name_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::DuplicateOptionName {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn duplicate_message_id(
        message_id: impl Into<String>,
        first_origin: Option<SourceSpan>,
        second_origin: Option<SourceSpan>,
    ) -> Self {
        Self::DuplicateMessageId {
            message_id: message_id.into(),
            first_origin: first_origin.map(Box::new),
            second_origin: second_origin.map(Box::new),
        }
    }

    pub(crate) fn missing_selector_annotation_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::MissingSelectorAnnotation {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn unknown_function_manifest(
        function: impl Into<String>,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::UnknownFunction {
            function: function.into(),
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn unsupported_function_usage(
        function: impl Into<String>,
        usage: &'static str,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::UnsupportedFunctionUsage {
            function: function.into(),
            usage,
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn unknown_function_option(
        function: impl Into<String>,
        option: impl Into<String>,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::UnknownFunctionOption {
            function: function.into(),
            option: option.into(),
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn invalid_function_option_value(
        function: impl Into<String>,
        option: impl Into<String>,
        expected: FunctionOptionValueKind,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::InvalidFunctionOptionValue {
            function: function.into(),
            option: option.into(),
            expected,
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn missing_function_option(
        function: impl Into<String>,
        option: impl Into<String>,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::MissingFunctionOption {
            function: function.into(),
            option: option.into(),
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn invalid_function_operand(
        function: impl Into<String>,
        expected: FunctionOperandKind,
        found: impl Into<String>,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::InvalidFunctionOperand {
            function: function.into(),
            expected,
            found: found.into(),
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn invalid_builtin_option_value(
        function: impl Into<String>,
        option: impl Into<String>,
        expected: &'static str,
        found: impl Into<String>,
        message_id: impl Into<String>,
        origin: Option<SourceSpan>,
    ) -> Self {
        Self::InvalidBuiltinOptionValue {
            function: function.into(),
            option: option.into(),
            expected,
            found: found.into(),
            message_id: Some(message_id.into()),
            origin: origin.map(Box::new),
        }
    }

    pub(crate) fn variant_key_mismatch_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::VariantKeyMismatch {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
        }
    }

    pub(crate) fn invalid_variant_key_detail(
        line: usize,
        column: usize,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        Self::InvalidVariantKey {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
            origin: None,
        }
    }

    pub(crate) fn invalid_variant_key_source(
        origin: Option<SourceSpan>,
        expected: &'static str,
        found: impl Into<String>,
    ) -> Self {
        let line = origin.as_ref().and_then(|origin| origin.line).unwrap_or(1);
        let column = origin
            .as_ref()
            .and_then(|origin| origin.column)
            .unwrap_or(1);
        Self::InvalidVariantKey {
            line,
            column,
            expected: Some(expected),
            found: Some(found.into()),
            origin: origin.map(Box::new),
        }
    }

    /// Render this error with an inline source snippet and caret.
    ///
    /// The source label defaults to `"<input>"`.
    #[must_use]
    pub fn render_with_source(&self, source: &str) -> String {
        self.render_with_source_label(source, "<input>")
    }

    /// Render this error with an inline source snippet and caret.
    ///
    /// `label` is shown in the location header (for example, a file path).
    #[must_use]
    pub fn render_with_source_label(&self, source: &str, label: &str) -> String {
        let Some(context) = self.diagnostic_context() else {
            return self.to_string();
        };
        let Some(line_text) = source.lines().nth(context.line.saturating_sub(1)) else {
            return self.to_string();
        };

        let line_width = context.line.to_string().len();
        let caret_pad = context.column.saturating_sub(1);
        let mut out = String::new();
        out.push_str(&format!("error: {}\n", context.title));
        out.push_str(&format!(
            " --> {}:{}:{}\n",
            label, context.line, context.column
        ));
        out.push_str(&format!("{:>line_width$} |\n", "", line_width = line_width));
        out.push_str(&format!(
            "{:>line_width$} | {}\n",
            context.line,
            line_text,
            line_width = line_width
        ));
        out.push_str(&format!(
            "{:>line_width$} | {}^",
            "",
            " ".repeat(caret_pad),
            line_width = line_width
        ));
        if let Some(expected) = context.expected {
            out.push_str(&format!(" expected {expected}"));
            if let Some(found) = context.found {
                out.push_str(&format!(", found {found}"));
            }
        } else if let Some(found) = context.found {
            out.push_str(&format!(" found {found}"));
        }
        out
    }

    /// Render this error without any appended message-id or origin suffixes.
    #[must_use]
    pub(crate) fn render_brief(&self) -> String {
        match self {
            Self::UnknownFunction { function, .. } => format!("unknown function {function:?}"),
            Self::UnsupportedFunctionUsage {
                function, usage, ..
            } => {
                format!("function {function:?} is not allowed in {usage} position")
            }
            Self::UnknownFunctionOption {
                function, option, ..
            } => {
                format!("unknown option {option:?} for function {function:?}")
            }
            Self::InvalidFunctionOptionValue {
                function,
                option,
                expected,
                ..
            } => {
                format!("option {option:?} for function {function:?} requires {expected} value")
            }
            Self::MissingFunctionOption {
                function, option, ..
            } => {
                format!("missing required option {option:?} for function {function:?}")
            }
            Self::InvalidBuiltinOptionValue {
                function,
                option,
                expected,
                found,
                ..
            } => format!(
                "option {option:?} for builtin {function:?} requires one of {expected}, found {found:?}"
            ),
            Self::InvalidFunctionOperand {
                function,
                expected,
                found,
                ..
            } => {
                format!("function {function:?} requires {expected} literal operand, found {found}")
            }
            _ => self.to_string(),
        }
    }

    /// Extract structured diagnostic location when available.
    ///
    /// Returns `None` for errors that lack a source position (e.g.
    /// [`InternalError`](Self::InternalError), [`TooManyStrings`](Self::TooManyStrings)).
    pub fn diagnostic_context(&self) -> Option<DiagnosticContext<'_>> {
        match self {
            Self::InvalidLine {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "invalid message line",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::EmptyMessageId {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "empty message id",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::InvalidExpr {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "invalid expression",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::InvalidVar {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "invalid variable",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::MissingDefaultArm {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "missing default arm",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::DuplicateDeclaration {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "duplicate declaration",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::DuplicateVariant {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "duplicate variant",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::DuplicateOptionName {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "duplicate option name",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::DuplicateMessageId { .. } => None,
            Self::UnknownFunction {
                function, origin, ..
            } => origin.as_deref().and_then(|origin| {
                Some(DiagnosticContext {
                    title: "unknown function",
                    line: origin.line?,
                    column: origin.column?,
                    expected: None,
                    found: Some(function.as_str()),
                })
            }),
            Self::UnsupportedFunctionUsage {
                function,
                usage,
                origin,
                ..
            } => origin.as_deref().and_then(|origin| {
                Some(DiagnosticContext {
                    title: "unsupported function usage",
                    line: origin.line?,
                    column: origin.column?,
                    expected: Some(*usage),
                    found: Some(function.as_str()),
                })
            }),
            Self::UnknownFunctionOption { option, origin, .. } => {
                origin.as_deref().and_then(|origin| {
                    Some(DiagnosticContext {
                        title: "unknown function option",
                        line: origin.line?,
                        column: origin.column?,
                        expected: None,
                        found: Some(option.as_str()),
                    })
                })
            }
            Self::InvalidFunctionOptionValue { option, origin, .. } => {
                origin.as_deref().and_then(|origin| {
                    Some(DiagnosticContext {
                        title: "invalid function option value",
                        line: origin.line?,
                        column: origin.column?,
                        expected: None,
                        found: Some(option.as_str()),
                    })
                })
            }
            Self::InvalidBuiltinOptionValue {
                expected,
                found,
                origin,
                ..
            } => origin.as_deref().and_then(|origin| {
                Some(DiagnosticContext {
                    title: "invalid builtin option value",
                    line: origin.line?,
                    column: origin.column?,
                    expected: Some(*expected),
                    found: Some(found.as_str()),
                })
            }),
            Self::MissingFunctionOption { option, origin, .. } => {
                origin.as_deref().and_then(|origin| {
                    Some(DiagnosticContext {
                        title: "missing function option",
                        line: origin.line?,
                        column: origin.column?,
                        expected: None,
                        found: Some(option.as_str()),
                    })
                })
            }
            Self::InvalidFunctionOperand { found, origin, .. } => {
                origin.as_deref().and_then(|origin| {
                    Some(DiagnosticContext {
                        title: "invalid function operand",
                        line: origin.line?,
                        column: origin.column?,
                        expected: None,
                        found: Some(found.as_str()),
                    })
                })
            }
            Self::MissingSelectorAnnotation {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "missing selector annotation",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::VariantKeyMismatch {
                line,
                column,
                expected,
                found,
            } => Some(DiagnosticContext {
                title: "variant key mismatch",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::InvalidVariantKey {
                line,
                column,
                expected,
                found,
                ..
            } => Some(DiagnosticContext {
                title: "invalid variant key",
                line: *line,
                column: *column,
                expected: expected.as_ref().copied(),
                found: found.as_deref(),
            }),
            Self::IoError { .. }
            | Self::FunctionIdOverflow
            | Self::TooManyStrings
            | Self::SizeOverflow { .. }
            | Self::ResourceInputError { .. }
            | Self::AliasResolutionOverflow { .. }
            | Self::InternalError { .. } => None,
        }
    }
}

/// Structured diagnostic location extracted from a [`CompileError`].
#[derive(Debug, Clone)]
pub struct DiagnosticContext<'a> {
    /// Short human-readable error category.
    pub title: &'static str,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
    /// What the parser expected at this position.
    pub expected: Option<&'a str>,
    /// What the parser actually found.
    pub found: Option<&'a str>,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLine {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "invalid message line",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::EmptyMessageId {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "empty message id",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::InvalidExpr {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "invalid expression",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::InvalidVar {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "invalid variable",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::MissingDefaultArm {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "missing default arm",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::DuplicateDeclaration {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "duplicate declaration",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::DuplicateVariant {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "duplicate variant",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::DuplicateOptionName {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "duplicate option name",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::DuplicateMessageId {
                message_id,
                first_origin,
                second_origin,
            } => write!(
                f,
                "duplicate message id {message_id:?}{}{}",
                format_origin_suffix(" first seen at ", first_origin.as_deref()),
                format_origin_suffix(" and again at ", second_origin.as_deref())
            ),
            Self::UnknownFunction {
                function,
                message_id,
                origin,
            } => write!(
                f,
                "unknown function {function:?}{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::UnsupportedFunctionUsage {
                function,
                usage,
                message_id,
                origin,
            } => write!(
                f,
                "function {function:?} is not allowed in {usage} position{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::UnknownFunctionOption {
                function,
                option,
                message_id,
                origin,
            } => write!(
                f,
                "unknown option {option:?} for function {function:?}{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::InvalidFunctionOptionValue {
                function,
                option,
                expected,
                message_id,
                origin,
            } => write!(
                f,
                "option {option:?} for function {function:?} requires {expected} value{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::MissingFunctionOption {
                function,
                option,
                message_id,
                origin,
            } => write!(
                f,
                "missing required option {option:?} for function {function:?}{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::InvalidBuiltinOptionValue {
                function,
                option,
                expected,
                found,
                message_id,
                origin,
            } => write!(
                f,
                "option {option:?} for builtin {function:?} requires one of {expected}, found {found:?}{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::InvalidFunctionOperand {
                function,
                expected,
                found,
                message_id,
                origin,
            } => write!(
                f,
                "function {function:?} requires {expected} literal operand, found {found}{}{}",
                format_message_suffix(message_id.as_deref()),
                format_origin_suffix(" at ", origin.as_deref())
            ),
            Self::MissingSelectorAnnotation {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "missing selector annotation",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::VariantKeyMismatch {
                line,
                column,
                expected,
                found,
            } => write_diagnostic(
                f,
                "variant key mismatch",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::InvalidVariantKey {
                line,
                column,
                expected,
                found,
                ..
            } => write_diagnostic(
                f,
                "invalid variant key",
                *line,
                *column,
                expected.as_ref().copied(),
                found.as_ref(),
            ),
            Self::IoError { path, source } => {
                write!(f, "I/O error at {}: {source}", path.display())
            }
            Self::FunctionIdOverflow => write!(f, "function id overflow"),
            Self::TooManyStrings => write!(f, "too many interned strings"),
            Self::SizeOverflow { what } => write!(f, "{what} overflow"),
            Self::ResourceInputError { detail } => write!(f, "invalid resource input: {detail}"),
            Self::AliasResolutionOverflow { alias } => {
                write!(f, "alias resolution exceeded maximum depth for {alias:?}")
            }
            Self::InternalError { detail } => write!(f, "internal compiler error: {detail}"),
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl CompileError {
    pub(crate) fn primary_source_id(&self) -> Option<crate::semantic::SourceId> {
        match self {
            Self::DuplicateMessageId {
                second_origin,
                first_origin,
                ..
            } => second_origin
                .as_deref()
                .or(first_origin.as_deref())
                .map(|origin| origin.source_id),
            Self::UnknownFunction { origin, .. }
            | Self::UnsupportedFunctionUsage { origin, .. }
            | Self::UnknownFunctionOption { origin, .. }
            | Self::InvalidFunctionOptionValue { origin, .. }
            | Self::InvalidBuiltinOptionValue { origin, .. }
            | Self::MissingFunctionOption { origin, .. }
            | Self::InvalidFunctionOperand { origin, .. }
            | Self::InvalidVariantKey { origin, .. } => {
                origin.as_deref().map(|origin| origin.source_id)
            }
            _ => None,
        }
    }

    pub(crate) fn primary_message_id(&self) -> Option<&str> {
        match self {
            Self::DuplicateMessageId { .. } => None,
            Self::UnknownFunction { message_id, .. }
            | Self::UnsupportedFunctionUsage { message_id, .. }
            | Self::UnknownFunctionOption { message_id, .. }
            | Self::InvalidFunctionOptionValue { message_id, .. }
            | Self::InvalidBuiltinOptionValue { message_id, .. }
            | Self::MissingFunctionOption { message_id, .. }
            | Self::InvalidFunctionOperand { message_id, .. } => message_id.as_deref(),
            _ => None,
        }
    }
}

fn write_diagnostic(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    line: usize,
    column: usize,
    expected: Option<&'static str>,
    found: Option<&String>,
) -> fmt::Result {
    write!(f, "{label} at {line}:{column}")?;
    if let Some(expected) = expected {
        write!(f, " (expected {expected}")?;
        if let Some(found) = found {
            write!(f, ", found {found}")?;
        }
        write!(f, ")")?;
    } else if let Some(found) = found {
        write!(f, " (found {found})")?;
    }
    Ok(())
}

fn format_origin_suffix(prefix: &str, origin: Option<&SourceSpan>) -> String {
    let Some(origin) = origin else {
        return String::new();
    };
    let mut out = String::from(prefix);
    out.push_str(&format!("source#{}", origin.source_id.0));
    if let Some(line) = origin.line {
        out.push(':');
        out.push_str(&line.to_string());
        if let Some(column) = origin.column {
            out.push(':');
            out.push_str(&column.to_string());
        }
    }
    out.push_str(&format!(
        " (bytes {}..{})",
        origin.byte_start, origin.byte_end
    ));
    out
}

fn format_message_suffix(message_id: Option<&str>) -> String {
    let Some(message_id) = message_id else {
        return String::new();
    };
    format!(" in message {message_id:?}")
}
