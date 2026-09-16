// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable JSON-facing observations for differential testing.

use std::collections::BTreeMap;

use message_format::{
    Locale,
    compiler::{CompileError, CompileOptions, compile},
    runtime::{
        Catalog, FormatDirection, FormatError, FormatOption, FormatSink, FormattedValue,
        FormattedValueKind, Formatter, MarkupKind, MessageFunctionError, Value,
    },
};
use serde::{Deserialize, Serialize};

/// One implementation-independent formatting input.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DifferentialCase {
    /// Stable identifier reported with a disagreement.
    pub id: String,
    /// MF2 source text.
    pub src: String,
    /// Formatting locale.
    #[serde(default = "default_locale")]
    pub locale: String,
    /// Bidi isolation strategy, either `default` or `none`.
    #[serde(default = "default_bidi_isolation")]
    pub bidi_isolation: String,
    /// Scalar external parameters.
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// Normalized result from one implementation.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DifferentialObservation {
    /// Stable identifier copied from the input case.
    pub id: String,
    /// Processing stage reached by the implementation.
    pub phase: DifferentialPhase,
    /// Rendered output for a successfully initialized formatter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Normalized diagnostic types.
    pub errors: Vec<String>,
    /// Structured output parts, when formatting ran.
    pub parts: Vec<serde_json::Value>,
}

/// Stage at which an observation completed.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DifferentialPhase {
    /// Source compilation was rejected.
    Compile,
    /// Input parameters could not be represented by this harness.
    Input,
    /// Message formatting ran.
    Format,
    /// Catalog or formatter initialization failed unexpectedly.
    Internal,
}

/// Observe one case using the Rust compiler and built-in runtime host.
#[must_use]
pub fn observe(case: &DifferentialCase) -> DifferentialObservation {
    let options = CompileOptions {
        default_bidi_isolation: case.bidi_isolation == "default",
        ..CompileOptions::default()
    };
    let bytes = match compile(&case.src, options) {
        Ok(bytes) => bytes,
        Err(error) => {
            return observation(
                case,
                DifferentialPhase::Compile,
                None,
                vec![map_compile_error(&error).to_string()],
                Vec::new(),
            );
        }
    };
    let catalog = match Catalog::from_bytes(&bytes) {
        Ok(catalog) => catalog,
        Err(error) => return internal_observation(case, format!("catalog:{error:?}")),
    };
    let locale = match case.locale.parse::<Locale>() {
        Ok(locale) => locale,
        Err(error) => return internal_observation(case, format!("locale:{error:?}")),
    };
    let host = match message_format::runtime::BuiltinHost::new(&locale) {
        Ok(host) => host,
        Err(error) => return internal_observation(case, format!("host:{error:?}")),
    };
    let mut formatter = match Formatter::new(&catalog, host) {
        Ok(formatter) => formatter,
        Err(error) => return internal_observation(case, format!("formatter:{error:?}")),
    };
    let args = match differential_args(&catalog, &case.params) {
        Ok(args) => args,
        Err(error) => {
            return observation(
                case,
                DifferentialPhase::Input,
                None,
                vec![error],
                Vec::new(),
            );
        }
    };
    let message = match formatter.resolve("main") {
        Ok(message) => message,
        Err(error) => {
            return observation(
                case,
                DifferentialPhase::Format,
                None,
                map_format_errors(&error)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                Vec::new(),
            );
        }
    };
    let mut sink = PartsSink::default();
    let mut errors = Vec::new();
    if let Err(error) = formatter.format_to(message, &args, &mut sink, Some(&mut errors)) {
        errors.push(error);
    }
    observation(
        case,
        DifferentialPhase::Format,
        Some(sink.output),
        errors
            .iter()
            .flat_map(map_format_errors)
            .map(str::to_string)
            .collect(),
        sink.parts,
    )
}

fn default_locale() -> String {
    "en-US".to_string()
}

fn default_bidi_isolation() -> String {
    "none".to_string()
}

fn observation(
    case: &DifferentialCase,
    phase: DifferentialPhase,
    output: Option<String>,
    errors: Vec<String>,
    parts: Vec<serde_json::Value>,
) -> DifferentialObservation {
    DifferentialObservation {
        id: case.id.clone(),
        phase,
        output,
        errors,
        parts,
    }
}

fn internal_observation(case: &DifferentialCase, error: String) -> DifferentialObservation {
    observation(
        case,
        DifferentialPhase::Internal,
        None,
        vec![error],
        Vec::new(),
    )
}

fn differential_args(
    catalog: &Catalog,
    params: &BTreeMap<String, serde_json::Value>,
) -> Result<Vec<(u32, Value)>, String> {
    params
        .iter()
        .filter_map(|(name, value)| {
            let value = match value {
                serde_json::Value::String(value) => Value::Str(value.clone()),
                serde_json::Value::Bool(value) => Value::Bool(*value),
                serde_json::Value::Null => Value::Null,
                serde_json::Value::Number(value) => value.as_i64().map_or_else(
                    || Value::Float(value.as_f64().unwrap_or_default()),
                    Value::Int,
                ),
                serde_json::Value::Object(value)
                    if value
                        .get("$datetime")
                        .is_some_and(serde_json::Value::is_string) =>
                {
                    Value::Str(value["$datetime"].as_str().unwrap_or_default().to_string())
                }
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                    return Some(Err(format!("unsupported-parameter:{name}")));
                }
            };
            catalog.string_id(name).map(|id| Ok((id, value)))
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct PartsSink {
    pub(crate) output: String,
    pub(crate) parts: Vec<serde_json::Value>,
}

impl FormatSink for PartsSink {
    fn wants_structured_output(&self) -> bool {
        true
    }

    fn literal(&mut self, value: &str) {
        self.output.push_str(value);
        self.parts
            .push(serde_json::json!({ "type": "text", "value": value }));
    }

    fn expression(&mut self, value: &str) {
        self.output.push_str(value);
        self.parts
            .push(serde_json::json!({ "type": "string", "value": value }));
    }

    fn markup_open(&mut self, name: &str, options: &[FormatOption<'_>]) {
        self.markup(MarkupKind::Open, name, None, options);
    }

    fn markup_close(&mut self, name: &str, options: &[FormatOption<'_>]) {
        self.markup(MarkupKind::Close, name, None, options);
    }

    fn formatted_value(&mut self, value: &FormattedValue<'_>) {
        self.output.push_str(&value.value);
        let mut part = serde_json::Map::new();
        part.insert(
            "type".to_string(),
            serde_json::Value::String(
                match value.kind {
                    FormattedValueKind::String => "string",
                    FormattedValueKind::Number => "number",
                    FormattedValueKind::DateTime => "datetime",
                }
                .to_string(),
            ),
        );
        if value.kind == FormattedValueKind::String {
            part.insert(
                "value".to_string(),
                serde_json::Value::String(value.value.to_string()),
            );
        }
        if let Some(locale) = &value.locale {
            part.insert(
                "locale".to_string(),
                serde_json::Value::String(locale.to_string()),
            );
        }
        if let Some(id) = value.id {
            part.insert("id".to_string(), serde_json::Value::String(id.to_string()));
        }
        if let Some(direction) = value.direction {
            let direction = match direction {
                FormatDirection::LeftToRight => "ltr",
                FormatDirection::RightToLeft => "rtl",
            };
            part.insert(
                "dir".to_string(),
                serde_json::Value::String(direction.to_string()),
            );
        }
        if !value.fields.is_empty() {
            part.insert(
                "parts".to_string(),
                serde_json::Value::Array(
                    value
                        .fields
                        .iter()
                        .map(
                            |field| serde_json::json!({ "type": field.kind, "value": field.value }),
                        )
                        .collect(),
                ),
            );
        }
        self.parts.push(serde_json::Value::Object(part));
    }

    fn bidi_isolation(&mut self, value: &str) {
        self.output.push_str(value);
        self.parts
            .push(serde_json::json!({ "type": "bidiIsolation", "value": value }));
    }

    fn fallback(&mut self, source: &str, rendered: &str) {
        self.output.push_str(rendered);
        self.parts
            .push(serde_json::json!({ "type": "fallback", "source": source }));
    }

    fn markup(
        &mut self,
        kind: MarkupKind,
        name: &str,
        id: Option<&str>,
        options: &[FormatOption<'_>],
    ) {
        let mut part = serde_json::Map::new();
        part.insert(
            "type".to_string(),
            serde_json::Value::String("markup".to_string()),
        );
        let kind = match kind {
            MarkupKind::Open => "open",
            MarkupKind::Close => "close",
            MarkupKind::Standalone => "standalone",
        };
        part.insert(
            "kind".to_string(),
            serde_json::Value::String(kind.to_string()),
        );
        part.insert(
            "name".to_string(),
            serde_json::Value::String(name.to_string()),
        );
        if let Some(id) = id {
            part.insert("id".to_string(), serde_json::Value::String(id.to_string()));
        }
        if !options.is_empty() {
            let options = options
                .iter()
                .filter(|option| option.key != "u:id")
                .map(|option| {
                    (
                        option.key.to_string(),
                        serde_json::Value::String(option.value.to_string()),
                    )
                })
                .collect();
            part.insert("options".to_string(), serde_json::Value::Object(options));
        }
        self.parts.push(serde_json::Value::Object(part));
    }
}

pub(crate) fn map_compile_error(error: &CompileError) -> &'static str {
    match error {
        CompileError::InvalidLine { .. }
        | CompileError::EmptyMessageId { .. }
        | CompileError::InvalidExpr { .. }
        | CompileError::InvalidVar { .. } => "syntax-error",
        CompileError::MissingDefaultArm { .. } => "missing-fallback-variant",
        CompileError::DuplicateDeclaration { .. } | CompileError::DuplicateMessageId { .. } => {
            "duplicate-declaration"
        }
        CompileError::DuplicateVariant { .. } => "duplicate-variant",
        CompileError::DuplicateOptionName { .. } => "duplicate-option-name",
        CompileError::UnknownFunction { .. } | CompileError::UnsupportedFunctionUsage { .. } => {
            "unknown-function"
        }
        CompileError::UnknownFunctionOption { .. }
        | CompileError::InvalidFunctionOptionValue { .. }
        | CompileError::InvalidBuiltinOptionValue { .. }
        | CompileError::MissingFunctionOption { .. } => "bad-option",
        CompileError::InvalidFunctionOperand { .. } => "bad-operand",
        CompileError::InvalidVariantKey { .. } => "bad-variant-key",
        CompileError::MissingSelectorAnnotation { .. } => "missing-selector-annotation",
        CompileError::VariantKeyMismatch { .. } => "variant-key-mismatch",
        CompileError::IoError { .. }
        | CompileError::FunctionIdOverflow
        | CompileError::TooManyStrings
        | CompileError::SizeOverflow { .. }
        | CompileError::ResourceInputError { .. }
        | CompileError::InternalError { .. } => "data-model-error",
    }
}

pub(crate) fn map_format_errors(error: &FormatError) -> Vec<&'static str> {
    let mut mapped = Vec::new();
    map_format_error_chain(error, &mut mapped);
    mapped
}

fn map_format_error_chain(error: &FormatError, mapped: &mut Vec<&'static str>) {
    match error {
        FormatError::UnknownFunction { .. } => mapped.push("unknown-function"),
        FormatError::MissingArg(_) => mapped.push("unresolved-variable"),
        FormatError::UnknownMessageId(_)
        | FormatError::StackUnderflow
        | FormatError::BadPc { .. }
        | FormatError::Decode(_)
        | FormatError::Trap(_) => mapped.push("data-model-error"),
        FormatError::Function(MessageFunctionError::BadOption) => mapped.push("bad-option"),
        FormatError::Function(MessageFunctionError::BadOperand) => mapped.push("bad-operand"),
        FormatError::Function(MessageFunctionError::UnsupportedOperation(_)) => {
            mapped.push("unsupported-operation");
        }
        FormatError::Function(MessageFunctionError::Implementation(_)) => {
            mapped.push("message-function-error");
        }
        FormatError::BadSelector { source } => {
            mapped.push("bad-selector");
            if let Some(source) = source {
                map_format_error_chain(source, mapped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_formatted_output_and_parts() {
        let case = DifferentialCase {
            id: "basic".to_string(),
            src: "Hello {$name}".to_string(),
            locale: default_locale(),
            bidi_isolation: default_bidi_isolation(),
            params: BTreeMap::from([(
                "name".to_string(),
                serde_json::Value::String("world".to_string()),
            )]),
        };

        let observed = observe(&case);

        assert_eq!(observed.phase, DifferentialPhase::Format);
        assert_eq!(observed.output.as_deref(), Some("Hello world"));
        assert!(observed.errors.is_empty());
        assert_eq!(observed.parts.len(), 2);
    }

    #[test]
    fn observes_compile_rejection() {
        let case = DifferentialCase {
            id: "invalid".to_string(),
            src: "{".to_string(),
            locale: default_locale(),
            bidi_isolation: default_bidi_isolation(),
            params: BTreeMap::new(),
        };

        let observed = observe(&case);

        assert_eq!(observed.phase, DifferentialPhase::Compile);
        assert_eq!(observed.errors, ["syntax-error"]);
    }
}
