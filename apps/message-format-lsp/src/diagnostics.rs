// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Conversion from compiler errors to LSP diagnostics.

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use message_format_compiler::{BuildError, CompileError, ResourceSpan};

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Map compiler severity to LSP severity.
fn map_severity(severity: message_format_compiler::DiagnosticSeverity) -> DiagnosticSeverity {
    match severity {
        message_format_compiler::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
        message_format_compiler::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
        message_format_compiler::DiagnosticSeverity::Note => DiagnosticSeverity::INFORMATION,
        message_format_compiler::DiagnosticSeverity::Help => DiagnosticSeverity::HINT,
    }
}

fn format_diagnostic_message(title: &str, expected: Option<&str>, found: Option<&str>) -> String {
    let mut msg = title.to_string();
    if let Some(expected) = expected {
        msg.push_str(": expected ");
        msg.push_str(expected);
        if let Some(found) = found {
            msg.push_str(", found ");
            msg.push_str(found);
        }
    } else if let Some(found) = found {
        msg.push_str(": ");
        msg.push_str(found);
    }
    msg
}

/// Convert a [`CompileError`] into an LSP [`Diagnostic`].
///
/// Errors that lack source-position context (e.g. `InternalError`) are placed
/// at the beginning of the document.
pub(crate) fn compile_error_to_diagnostic(error: &CompileError) -> Diagnostic {
    let (range, message) = match error.diagnostic_context() {
        Some(ctx) => {
            let line = saturating_u32(ctx.line.saturating_sub(1));
            let col = saturating_u32(ctx.column.saturating_sub(1));
            let pos = Position::new(line, col);
            (
                Range::new(pos, pos),
                format_diagnostic_message(ctx.title, ctx.expected, ctx.found),
            )
        }
        None => {
            let pos = Position::new(0, 0);
            (Range::new(pos, pos), error.to_string())
        }
    };

    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(String::from("mf2")),
        message,
        ..Diagnostic::default()
    }
}

/// Convert a [`BuildError`] into an LSP [`Diagnostic`].
///
/// When the error carries container-level context (resource origin), the
/// diagnostic combines the container position with the body-relative error
/// position. The origin column is adjusted by +1 to skip the opening quote
/// of the string token recorded by the resource adapters.
pub(crate) fn build_error_to_diagnostic(error: &BuildError) -> Diagnostic {
    let severity = Some(map_severity(error.severity));

    if let Some(context) = &error.context {
        if let Some(origin) = &context.message_origin {
            let origin_line = saturating_u32(origin.line.unwrap_or(1).saturating_sub(1));
            // +1 to skip past the opening quote of the string token.
            let origin_col = saturating_u32(origin.column.unwrap_or(1));

            let (pos, message) = if let Some(diag_ctx) = error.error.diagnostic_context() {
                let body_line = saturating_u32(diag_ctx.line.saturating_sub(1));
                let body_col = saturating_u32(diag_ctx.column.saturating_sub(1));

                // For the first line of the body, add origin column offset;
                // for subsequent lines, the column is body-relative only.
                let line = origin_line + body_line;
                let col = if body_line == 0 {
                    origin_col + body_col
                } else {
                    body_col
                };

                let title = format!("{}: {}", context.message_id, diag_ctx.title);
                let msg = format_diagnostic_message(&title, diag_ctx.expected, diag_ctx.found);
                (Position::new(line, col), msg)
            } else {
                let pos = Position::new(origin_line, origin_col);
                let msg = format!("{}: {}", context.message_id, error.error);
                (pos, msg)
            };

            return Diagnostic {
                range: Range::new(pos, pos),
                severity,
                source: Some(String::from("mf2")),
                message,
                ..Diagnostic::default()
            };
        }
    }

    // Fall back to the inner compile error's position.
    let mut diag = compile_error_to_diagnostic(&error.error);
    diag.severity = severity;
    diag
}

/// Convert a resource adapter error message into an LSP [`Diagnostic`].
pub(crate) fn resource_error_to_diagnostic(
    message: &str,
    span: Option<&ResourceSpan>,
) -> Diagnostic {
    let pos = span.map_or_else(
        || Position::new(0, 0),
        |span| {
            Position::new(
                saturating_u32(span.line.unwrap_or(1).saturating_sub(1)),
                saturating_u32(span.column.unwrap_or(1).saturating_sub(1)),
            )
        },
    );
    Diagnostic {
        range: Range::new(pos, pos),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(String::from("mf2")),
        message: message.to_string(),
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::resource_error_to_diagnostic;
    use message_format_compiler::ResourceSpan;

    #[test]
    fn resource_error_uses_span_when_available() {
        let span = ResourceSpan {
            byte_start: 10,
            byte_end: 10,
            line: Some(3),
            column: Some(7),
        };
        let diagnostic = resource_error_to_diagnostic("bad resource", Some(&span));
        assert_eq!(diagnostic.range.start.line, 2);
        assert_eq!(diagnostic.range.start.character, 6);
    }

    #[test]
    fn resource_error_defaults_to_origin_when_no_span_exists() {
        let diagnostic = resource_error_to_diagnostic("bad resource", None);
        assert_eq!(diagnostic.range.start.line, 0);
        assert_eq!(diagnostic.range.start.character, 0);
    }
}
