// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! JSON resource adapters for [`message_format_compiler::ResourceInput`].
//!
//! JSON localization data is not one format. Different tools and ecosystems use
//! different object layouts, metadata conventions, and nesting rules. This
//! crate keeps that complexity at the adapter boundary by exposing explicit
//! JSON profiles instead of pretending all JSON resource files are
//! interchangeable.
//!
//! Currently supported profiles:
//!
//! - [`JsonProfile::Flat`]
//!   A top-level object mapping each message id directly to one MF2 message
//!   string.
//!   Example:
//!   ```json
//!   {
//!     "app.title": "Example App",
//!     "user.greeting": "Hello { $name }"
//!   }
//!   ```
//! - [`JsonProfile::Chrome`]
//!   A Chrome-style `messages.json` object whose top-level keys map to objects
//!   with a required `"message"` string and optional metadata such as
//!   `"description"` or `"placeholders"`.
//!   Example:
//!   ```json
//!   {
//!     "cart.items": {
//!       "message": "{ $count :number } items",
//!       "description": "Number of items in the cart"
//!     }
//!   }
//!   ```
//!
//! Both profiles preserve container provenance for the message-value string
//! token itself, so compiler source maps can point back to the exact JSON value
//! that produced each message.

use std::fmt;

use json_spanned_value::{ErrorExt, spanned::Value as JsonValue};
use message_format_compiler::{MessageResource, ResourceInput, ResourceSpan, SourceKind};

/// Supported JSON resource/container profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonProfile {
    /// A flat top-level object: `{ "id": "message" }`.
    Flat,
    /// A Chrome-style `messages.json` object with `{ "id": { "message": ... } }` entries.
    Chrome,
}

impl JsonProfile {
    fn source_kind(self) -> SourceKind {
        match self {
            Self::Flat => SourceKind::Other(String::from("resource-json-flat")),
            Self::Chrome => SourceKind::Other(String::from("resource-json-chrome")),
        }
    }
}

/// Parse a JSON resource/container using one explicit [`JsonProfile`].
pub fn parse_json_resource(
    name: impl Into<String>,
    source: &str,
    profile: JsonProfile,
) -> Result<ResourceInput, ResourceJsonError> {
    let root: JsonValue = json_spanned_value::from_str(source)
        .map_err(|err| ResourceJsonError::from_json(source, err))?;
    match profile {
        JsonProfile::Flat => parse_flat_value(name, source, root),
        JsonProfile::Chrome => parse_chrome_value(name, source, root),
    }
}

/// Parse a flat JSON object like `{ "id": "message" }`.
#[must_use = "resource parsing results should be handled explicitly"]
pub fn parse_flat_json_resource(
    name: impl Into<String>,
    source: &str,
) -> Result<ResourceInput, ResourceJsonError> {
    parse_json_resource(name, source, JsonProfile::Flat)
}

/// Parse a Chrome-style `messages.json` resource document.
#[must_use = "resource parsing results should be handled explicitly"]
pub fn parse_chrome_json_resource(
    name: impl Into<String>,
    source: &str,
) -> Result<ResourceInput, ResourceJsonError> {
    parse_json_resource(name, source, JsonProfile::Chrome)
}

fn parse_flat_value(
    name: impl Into<String>,
    source: &str,
    root: JsonValue,
) -> Result<ResourceInput, ResourceJsonError> {
    let entries = root
        .into_span_object()
        .map_err(|root| ResourceJsonError::ExpectedObject {
            detail: String::from("top-level JSON value must be an object"),
            span: Some(span_from_json_value(source, &root)),
        })?;
    if entries.is_empty() {
        return Err(ResourceJsonError::MissingMessages);
    }

    let mut input = ResourceInput::new(name, JsonProfile::Flat.source_kind());
    for (id, value) in entries {
        let message_id = id.into_inner();
        let (message, origin) = string_message_entry(source, &message_id, value)?;
        input = input.resource(MessageResource::new(message_id, message).origin(origin));
    }
    Ok(input)
}

fn parse_chrome_value(
    name: impl Into<String>,
    source: &str,
    root: JsonValue,
) -> Result<ResourceInput, ResourceJsonError> {
    let entries = root
        .into_span_object()
        .map_err(|root| ResourceJsonError::ExpectedObject {
            detail: String::from("top-level JSON value must be an object"),
            span: Some(span_from_json_value(source, &root)),
        })?;
    if entries.is_empty() {
        return Err(ResourceJsonError::MissingMessages);
    }

    let mut input = ResourceInput::new(name, JsonProfile::Chrome.source_kind());
    for (id, value) in entries {
        let message_id = id.into_inner();
        let (message, origin) = chrome_message_entry(source, &message_id, value)?;
        input = input.resource(MessageResource::new(message_id, message).origin(origin));
    }
    Ok(input)
}

fn string_message_entry(
    source: &str,
    message_id: &str,
    value: JsonValue,
) -> Result<(String, ResourceSpan), ResourceJsonError> {
    let string = value
        .as_span_string()
        .ok_or_else(|| ResourceJsonError::ExpectedStringValue {
            message_id: message_id.to_string(),
            span: Some(span_from_json_value(source, &value)),
        })?;
    Ok((
        string.get_ref().to_string(),
        span_to_resource_span(source, string.start(), string.end()),
    ))
}

fn chrome_message_entry(
    source: &str,
    message_id: &str,
    value: JsonValue,
) -> Result<(String, ResourceSpan), ResourceJsonError> {
    let mut object =
        value
            .into_span_object()
            .map_err(|value| ResourceJsonError::ExpectedObject {
                detail: format!("message entry {message_id:?} must be an object"),
                span: Some(span_from_json_value(source, &value)),
            })?;
    let Some(value) = object.get_mut().remove("message") else {
        return Err(ResourceJsonError::MissingChromeMessage {
            message_id: message_id.to_string(),
            span: Some(span_to_resource_span(source, object.start(), object.end())),
        });
    };
    let string =
        value
            .as_span_string()
            .ok_or_else(|| ResourceJsonError::ExpectedChromeMessageString {
                message_id: message_id.to_string(),
                span: Some(span_from_json_value(source, &value)),
            })?;
    Ok((
        string.get_ref().to_string(),
        span_to_resource_span(source, string.start(), string.end()),
    ))
}

fn span_to_resource_span(source: &str, byte_start: usize, byte_end: usize) -> ResourceSpan {
    let (line, column) = line_and_column(source, byte_start);
    ResourceSpan {
        byte_start,
        byte_end,
        line: Some(line),
        column: Some(column),
    }
}

fn point_to_resource_span(source: &str, byte_offset: usize) -> ResourceSpan {
    let (line, column) = line_and_column(source, byte_offset);
    ResourceSpan {
        byte_start: byte_offset,
        byte_end: byte_offset,
        line: Some(line),
        column: Some(column),
    }
}

fn span_from_json_value(source: &str, value: &JsonValue) -> ResourceSpan {
    span_to_resource_span(source, value.start(), value.end())
}

fn line_and_column(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = source
        .get(..byte_offset)
        .expect("json span start must lie within the source");
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

/// JSON resource parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceJsonError {
    /// JSON decoding failed.
    InvalidJson {
        /// Human-readable parse detail.
        detail: String,
        /// Precise parser location when available.
        span: Option<ResourceSpan>,
    },
    /// Expected a JSON object in a specific position.
    ExpectedObject {
        /// Human-readable failure detail.
        detail: String,
        /// Span of the unexpected value when available.
        span: Option<ResourceSpan>,
    },
    /// The parsed document did not define any messages.
    MissingMessages,
    /// A flat JSON message value was not a string.
    ExpectedStringValue {
        /// Message id with the invalid value.
        message_id: String,
        /// Span of the invalid value when available.
        span: Option<ResourceSpan>,
    },
    /// A Chrome JSON message object was missing its `message` field.
    MissingChromeMessage {
        /// Message id with the missing field.
        message_id: String,
        /// Span of the containing message entry when available.
        span: Option<ResourceSpan>,
    },
    /// A Chrome JSON message object's `message` field was not a string.
    ExpectedChromeMessageString {
        /// Message id with the invalid field.
        message_id: String,
        /// Span of the invalid field value when available.
        span: Option<ResourceSpan>,
    },
}

impl ResourceJsonError {
    fn from_json(source: &str, error: impl fmt::Display + ErrorExt) -> Self {
        Self::InvalidJson {
            detail: error.to_string(),
            span: error
                .offset_within(source)
                .map(|offset| point_to_resource_span(source, offset)),
        }
    }

    /// Exact parser span when the adapter could determine one.
    #[must_use]
    pub fn span(&self) -> Option<&ResourceSpan> {
        match self {
            Self::InvalidJson { span, .. }
            | Self::ExpectedObject { span, .. }
            | Self::ExpectedStringValue { span, .. }
            | Self::MissingChromeMessage { span, .. }
            | Self::ExpectedChromeMessageString { span, .. } => span.as_ref(),
            Self::MissingMessages => None,
        }
    }
}

impl fmt::Display for ResourceJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson { detail, .. } => {
                write!(f, "invalid JSON resource document: {detail}")
            }
            Self::ExpectedObject { detail, .. } => write!(f, "{detail}"),
            Self::MissingMessages => {
                write!(f, "resource document must define at least one message")
            }
            Self::ExpectedStringValue { message_id, .. } => {
                write!(f, "message {message_id:?} must map to a string value")
            }
            Self::MissingChromeMessage { message_id, .. } => {
                write!(
                    f,
                    "message {message_id:?} must define a string \"message\" field"
                )
            }
            Self::ExpectedChromeMessageString { message_id, .. } => {
                write!(
                    f,
                    "message {message_id:?} has a non-string \"message\" field"
                )
            }
        }
    }
}

impl std::error::Error for ResourceJsonError {}

#[cfg(test)]
mod tests {
    use super::*;
    use message_format_compiler::{CompileOptions, SourceId, SourceSpan, compile_resources};

    #[test]
    fn parse_json_resource_dispatches_flat_profile() {
        let input = parse_json_resource(
            "app.json",
            r#"{ "app.title": "Example App" }"#,
            JsonProfile::Flat,
        )
        .expect("parse");

        assert_eq!(
            input.kind,
            SourceKind::Other(String::from("resource-json-flat"))
        );
        assert_eq!(input.resources.len(), 1);
    }

    #[test]
    fn parse_json_resource_dispatches_chrome_profile() {
        let input = parse_json_resource(
            "app.json",
            r#"{ "app.title": { "message": "Example App" } }"#,
            JsonProfile::Chrome,
        )
        .expect("parse");

        assert_eq!(
            input.kind,
            SourceKind::Other(String::from("resource-json-chrome"))
        );
        assert_eq!(input.resources.len(), 1);
    }

    #[test]
    fn parse_flat_json_resource_reads_string_values_and_spans() {
        let source = r#"{ "app.title": "Example App", "user.greeting": "Hello { $name }" }"#;
        let input = parse_flat_json_resource("app.json", source).expect("parse");

        assert_eq!(
            input.kind,
            SourceKind::Other(String::from("resource-json-flat"))
        );
        assert_eq!(input.resources.len(), 2);
        assert_eq!(input.resources[0].id, "app.title");
        assert_eq!(input.resources[0].source, "Example App");
        let origin = input.resources[0].origin.as_ref().expect("origin");
        assert_eq!(
            &source[origin.byte_start..origin.byte_end],
            "\"Example App\""
        );
        assert_eq!(origin.line, Some(1));
        assert_eq!(origin.column, Some(16));
    }

    #[test]
    fn parse_chrome_json_resource_reads_message_field_and_spans() {
        let source = r#"
{
  "cart.items": {
    "message": "{ $count :number } items",
    "description": "Number of items in the cart"
  }
}
"#;
        let input = parse_chrome_json_resource("app.json", source).expect("parse");

        assert_eq!(
            input.kind,
            SourceKind::Other(String::from("resource-json-chrome"))
        );
        assert_eq!(input.resources.len(), 1);
        assert_eq!(input.resources[0].id, "cart.items");
        assert_eq!(input.resources[0].source, "{ $count :number } items");
        let origin = input.resources[0].origin.as_ref().expect("origin");
        assert_eq!(
            &source[origin.byte_start..origin.byte_end],
            "\"{ $count :number } items\""
        );
        assert_eq!(origin.line, Some(4));
        assert_eq!(origin.column, Some(16));
    }

    #[test]
    fn chrome_json_requires_message_field() {
        let err =
            parse_chrome_json_resource("app.json", r#"{ "app.title": { "description": "..." } }"#)
                .expect_err("must fail");
        match err {
            ResourceJsonError::MissingChromeMessage { message_id, span } => {
                assert_eq!(message_id, "app.title");
                let span = span.expect("span");
                assert_eq!(span.line, Some(1));
                assert!(span.column.is_some_and(|column| column > 10));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn chrome_json_requires_entries() {
        let err = parse_chrome_json_resource("empty.json", "{}").expect_err("must fail");
        assert_eq!(err, ResourceJsonError::MissingMessages);
    }

    #[test]
    fn flat_json_type_error_carries_value_span() {
        let err =
            parse_flat_json_resource("app.json", r#"{ "app.title": 42 }"#).expect_err("must fail");
        match err {
            ResourceJsonError::ExpectedStringValue { message_id, span } => {
                assert_eq!(message_id, "app.title");
                let span = span.expect("span");
                assert_eq!(span.line, Some(1));
                assert_eq!(span.column, Some(16));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn invalid_json_carries_parser_span() {
        let err = parse_flat_json_resource("app.json", "{").expect_err("must fail");
        match err {
            ResourceJsonError::InvalidJson { span, .. } => {
                let span = span.expect("span");
                assert_eq!(span.line, Some(1));
                assert_eq!(span.column, Some(1));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parsed_json_resource_preserves_origin_through_compilation() {
        let input = parse_chrome_json_resource(
            "app.json",
            r#"{ "app.title": { "message": "Example App" } }"#,
        )
        .expect("parse");

        let compiled = compile_resources([input], CompileOptions::default())
            .into_result()
            .expect("compile");
        assert_eq!(
            compiled.source_map.messages[0].origin,
            Some(SourceSpan {
                source_id: SourceId(0),
                byte_start: 28,
                byte_end: 41,
                line: Some(1),
                column: Some(29),
            })
        );
    }
}
