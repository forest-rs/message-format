// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TOML resource adapter for [`message_format_compiler::ResourceInput`].

use std::fmt;

use message_format_compiler::{MessageResource, ResourceInput, ResourceSpan, SourceKind};
use toml::de::DeTable;

/// Parse one project-owned TOML resource container into a compiler [`ResourceInput`].
///
/// This format is intentionally a resource/container adapter, not MF2 itself.
/// It supports either inline values:
///
/// ```toml
/// [messages]
/// "app.title" = "Example App"
/// ```
///
/// or structured entries:
///
/// ```toml
/// [messages."cart.items"]
/// value = "{ $count :number } items"
/// description = "Number of items in the cart"
/// ```
pub fn parse_resource_toml(
    name: impl Into<String>,
    source: &str,
) -> Result<ResourceInput, ResourceTomlError> {
    let parsed = DeTable::parse(source).map_err(|err| ResourceTomlError::from_toml(source, err))?;
    let root = parsed.into_inner();
    let messages = root
        .get("messages")
        .and_then(|value| value.get_ref().as_table())
        .ok_or(ResourceTomlError::MissingMessages)?;
    if messages.is_empty() {
        return Err(ResourceTomlError::MissingMessages);
    }

    let mut input = ResourceInput::new(name, SourceKind::Other(String::from("resource-toml")));
    for (id, entry) in messages {
        let message_id = id.get_ref().to_string();
        let (value, origin) = message_value_from_entry(source, &message_id, entry)?;
        input = input.resource(MessageResource::new(message_id, value).origin(origin));
    }
    Ok(input)
}

fn message_value_from_entry(
    source: &str,
    message_id: &str,
    entry: &toml::Spanned<toml::de::DeValue<'_>>,
) -> Result<(String, ResourceSpan), ResourceTomlError> {
    if let Some(value) = entry.get_ref().as_str() {
        return Ok((
            value.to_string(),
            span_to_resource_span(source, entry.span()),
        ));
    }

    let table = entry.get_ref().as_table().ok_or_else(|| {
        ResourceTomlError::invalid_entry(
            message_id,
            "entry must be a string or table",
            Some(span_to_resource_span(source, entry.span())),
        )
    })?;
    let value = table.get("value").ok_or_else(|| {
        ResourceTomlError::invalid_entry(
            message_id,
            "structured entry must define a string `value` field",
            Some(span_to_resource_span(source, entry.span())),
        )
    })?;
    let text = value.get_ref().as_str().ok_or_else(|| {
        ResourceTomlError::invalid_entry(
            message_id,
            "`value` must be a string",
            Some(span_to_resource_span(source, value.span())),
        )
    })?;
    Ok((
        text.to_string(),
        span_to_resource_span(source, value.span()),
    ))
}

fn span_to_resource_span(source: &str, span: core::ops::Range<usize>) -> ResourceSpan {
    let (line, column) = line_and_column(source, span.start);
    ResourceSpan {
        byte_start: span.start,
        byte_end: span.end,
        line: Some(line),
        column: Some(column),
    }
}

fn line_and_column(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = source
        .get(..byte_offset)
        .expect("toml span start must lie within the source");
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

/// TOML resource parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceTomlError {
    /// TOML decoding failed.
    InvalidToml {
        /// Human-readable parse detail.
        detail: String,
        /// Exact parser location when available.
        span: Option<ResourceSpan>,
    },
    /// The parsed document did not define any messages.
    MissingMessages,
}

impl ResourceTomlError {
    fn from_toml(source: &str, error: toml::de::Error) -> Self {
        Self::InvalidToml {
            detail: error.to_string(),
            span: error.span().map(|span| span_to_resource_span(source, span)),
        }
    }

    fn invalid_entry(message_id: &str, detail: &'static str, span: Option<ResourceSpan>) -> Self {
        Self::InvalidToml {
            detail: format!("invalid message entry {message_id:?}: {detail}"),
            span,
        }
    }

    /// Exact parser span when the adapter could determine one.
    #[must_use]
    pub fn span(&self) -> Option<&ResourceSpan> {
        match self {
            Self::InvalidToml { span, .. } => span.as_ref(),
            Self::MissingMessages => None,
        }
    }
}

impl fmt::Display for ResourceTomlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToml { detail, .. } => {
                write!(f, "invalid TOML resource document: {detail}")
            }
            Self::MissingMessages => {
                write!(f, "resource document must define at least one message")
            }
        }
    }
}

impl std::error::Error for ResourceTomlError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resource_toml_supports_inline_and_structured_entries() {
        let input = parse_resource_toml(
            "app.toml",
            r#"
[messages]
"app.title" = "Example App"

[messages."cart.items"]
value = "{ $count :number } items"
description = "Number of items in the cart"
"#,
        )
        .expect("parse");

        assert_eq!(input.name, "app.toml");
        assert_eq!(input.kind, SourceKind::Other(String::from("resource-toml")));
        assert_eq!(input.resources.len(), 2);
        assert_eq!(input.resources[0].id, "app.title");
        assert_eq!(input.resources[0].source, "Example App");
        assert_eq!(
            input.resources[0]
                .origin
                .as_ref()
                .expect("inline origin")
                .line,
            Some(3)
        );
        assert_eq!(
            input.resources[0]
                .origin
                .as_ref()
                .expect("inline origin")
                .column,
            Some(15)
        );
        assert_eq!(input.resources[1].id, "cart.items");
        assert_eq!(input.resources[1].source, "{ $count :number } items");
        assert_eq!(
            input.resources[1]
                .origin
                .as_ref()
                .expect("structured origin")
                .line,
            Some(6)
        );
        assert_eq!(
            input.resources[1]
                .origin
                .as_ref()
                .expect("structured origin")
                .column,
            Some(9)
        );
    }

    #[test]
    fn parse_resource_toml_preserves_value_byte_spans() {
        let source = r#"
[messages]
"app.title" = "Example App"

[messages."cart.items"]
value = "{ $count :number } items"
"#;
        let input = parse_resource_toml("app.toml", source).expect("parse");

        let inline = input.resources[0].origin.as_ref().expect("inline origin");
        assert_eq!(
            &source[inline.byte_start..inline.byte_end],
            "\"Example App\""
        );

        let structured = input.resources[1]
            .origin
            .as_ref()
            .expect("structured origin");
        assert_eq!(
            &source[structured.byte_start..structured.byte_end],
            "\"{ $count :number } items\""
        );
    }

    #[test]
    fn parse_resource_toml_requires_messages() {
        let err = parse_resource_toml("empty.toml", "").expect_err("must fail");
        assert_eq!(err, ResourceTomlError::MissingMessages);
    }

    #[test]
    fn parse_resource_toml_invalid_entry_carries_span() {
        let err = parse_resource_toml(
            "app.toml",
            r#"
[messages]
"app.title" = 42
"#,
        )
        .expect_err("must fail");

        match err {
            ResourceTomlError::InvalidToml { span, .. } => {
                let span = span.expect("span");
                assert_eq!(span.line, Some(3));
                assert_eq!(span.column, Some(15));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_resource_toml_syntax_error_carries_span() {
        let err = parse_resource_toml("broken.toml", "[messages").expect_err("must fail");
        match err {
            ResourceTomlError::InvalidToml { span, .. } => {
                let span = span.expect("span");
                assert_eq!(span.line, Some(1));
                assert_eq!(span.column, Some(10));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
