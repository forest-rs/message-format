// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Host-agnostic function manifest types for compiler validation and tooling metadata.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// Manifest describing which function names and options are valid during compilation.
///
/// This is a tooling contract, not a runtime host implementation. Use it to
/// teach the compiler and CLI about custom functions without linking
/// executable host code into those tools.
///
/// Today the compiler and CLI use manifests for validation. The optional
/// metadata fields (`display_name`, `category`, `doc`, `snippet`) are carried
/// here so future editor/help surfaces can consume them, but those richer
/// tooling consumers do not exist yet in this workspace.
///
/// The manifest format is TOML:
///
/// ```toml
/// [functions.string]
/// format = true
/// select = true
///
/// [functions."app:link"]
/// format = true
/// operand = "string"
/// doc = "Open an in-app link"
/// snippet = "{ $value :app:link href=$url }"
///
/// [functions."app:link".options.href]
/// value = "either"
/// required = true
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FunctionManifest {
    functions: BTreeMap<String, FunctionSchema>,
}

impl FunctionManifest {
    /// Create an empty manifest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a TOML manifest.
    pub fn parse(source: &str) -> Result<Self, ManifestParseError> {
        let manifest = toml::from_str::<TomlManifest>(source)
            .map_err(|error| ManifestParseError::from_toml(source, error))?;
        Ok(manifest.into_manifest())
    }

    /// Insert or replace one function schema.
    pub fn insert(&mut self, schema: FunctionSchema) -> Option<FunctionSchema> {
        self.functions.insert(schema.name.clone(), schema)
    }

    /// Look up one function schema by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FunctionSchema> {
        self.functions.get(name)
    }

    /// Iterate the registered function schemas in name order.
    pub fn functions(&self) -> impl Iterator<Item = &FunctionSchema> {
        self.functions.values()
    }
}

/// One function schema entry in a [`FunctionManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSchema {
    /// Function identifier as written in source, without the leading `:`.
    pub name: String,
    /// Whether `{ ... :name }` formatting calls are allowed.
    pub allows_format: bool,
    /// Whether `.match` selector annotations may use this function.
    pub allows_select: bool,
    /// Which literal operand forms the compiler may validate for this function.
    pub operand_kind: FunctionOperandKind,
    /// Which selector-key forms are valid when this function is used in `.match`.
    pub selector_keys: FunctionSelectorKeyKind,
    /// Whether option keys beyond [`Self::options`] are accepted.
    pub allow_unknown_options: bool,
    /// Optional human-friendly label for editor or CLI help surfaces.
    pub display_name: Option<String>,
    /// Optional category label for grouping related functions in tooling.
    pub category: Option<String>,
    /// Optional user-facing documentation for the function.
    pub doc: Option<String>,
    /// Optional snippet template for editor completions.
    pub snippet: Option<String>,
    options: BTreeMap<String, FunctionOptionSchema>,
}

impl FunctionSchema {
    /// Create a new schema with no allowed call sites or options.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            allows_format: false,
            allows_select: false,
            operand_kind: FunctionOperandKind::Any,
            selector_keys: FunctionSelectorKeyKind::Any,
            allow_unknown_options: false,
            display_name: None,
            category: None,
            doc: None,
            snippet: None,
            options: BTreeMap::new(),
        }
    }

    /// Mark this function as valid in ordinary formatting expressions.
    #[must_use]
    pub fn allow_format(mut self) -> Self {
        self.allows_format = true;
        self
    }

    /// Mark this function as valid in `.match` selector position.
    #[must_use]
    pub fn allow_select(mut self) -> Self {
        self.allows_select = true;
        self
    }

    /// Constrain which literal operands the compiler accepts for this function.
    #[must_use]
    pub fn operand_kind(mut self, operand_kind: FunctionOperandKind) -> Self {
        self.operand_kind = operand_kind;
        self
    }

    /// Constrain which variant-key forms are accepted for `.match` arms.
    #[must_use]
    pub fn selector_keys(mut self, selector_keys: FunctionSelectorKeyKind) -> Self {
        self.selector_keys = selector_keys;
        self
    }

    /// Permit option keys beyond those explicitly registered on the schema.
    #[must_use]
    pub fn allow_unknown_options(mut self) -> Self {
        self.allow_unknown_options = true;
        self
    }

    /// Set an optional human-friendly label for tooling.
    #[must_use]
    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Set an optional category label for tooling.
    #[must_use]
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Set optional user-facing documentation for tooling.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = Some(doc.into());
        self
    }

    /// Set an optional completion snippet for tooling.
    #[must_use]
    pub fn snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Register one option on the schema.
    #[must_use]
    pub fn option(mut self, key: impl Into<String>, value_kind: FunctionOptionValueKind) -> Self {
        let key = key.into();
        self.options.insert(
            key.clone(),
            FunctionOptionSchema {
                key,
                value_kind,
                required: false,
            },
        );
        self
    }

    /// Register one required option on the schema.
    #[must_use]
    pub fn required_option(
        mut self,
        key: impl Into<String>,
        value_kind: FunctionOptionValueKind,
    ) -> Self {
        let key = key.into();
        self.options.insert(
            key.clone(),
            FunctionOptionSchema {
                key,
                value_kind,
                required: true,
            },
        );
        self
    }

    /// Look up one registered option by key.
    #[must_use]
    pub fn option_schema(&self, key: &str) -> Option<&FunctionOptionSchema> {
        self.options.get(key)
    }

    /// Iterate registered options in key order.
    pub fn options(&self) -> impl Iterator<Item = &FunctionOptionSchema> {
        self.options.values()
    }
}

/// One option rule on a [`FunctionSchema`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionOptionSchema {
    /// Option key.
    pub key: String,
    /// Which value forms are accepted.
    pub value_kind: FunctionOptionValueKind,
    /// Whether this option must be present on every use of the function.
    pub required: bool,
}

/// Literal operand kinds the compiler can validate from source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionOperandKind {
    /// No compile-time operand restriction.
    Any,
    /// Any string literal or variable operand is allowed.
    String,
    /// Literal operands must be valid MF2 number literals.
    Number,
}

impl fmt::Display for FunctionOperandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "any"),
            Self::String => write!(f, "string"),
            Self::Number => write!(f, "number"),
        }
    }
}

/// Allowed value shapes for one function option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionOptionValueKind {
    /// Only literal option values are accepted.
    Literal,
    /// Only variable option values are accepted.
    Variable,
    /// Both literal and variable option values are accepted.
    Either,
}

impl FunctionOptionValueKind {
    #[must_use]
    pub(crate) const fn accepts_literal(self) -> bool {
        matches!(self, Self::Literal | Self::Either)
    }

    #[must_use]
    pub(crate) const fn accepts_variable(self) -> bool {
        matches!(self, Self::Variable | Self::Either)
    }
}

/// Accepted `.match` key forms for one selector function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FunctionSelectorKeyKind {
    /// Do not perform compile-time selector-key validation.
    Any,
    /// Keys must be identifiers.
    Identifier,
    /// Keys must be MF2 number literals.
    Number,
    /// Keys must be CLDR plural categories.
    PluralCategory,
    /// Keys may be either CLDR plural categories or MF2 number literals.
    NumberOrPluralCategory,
}

impl fmt::Display for FunctionSelectorKeyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "any"),
            Self::Identifier => write!(f, "identifier"),
            Self::Number => write!(f, "number"),
            Self::PluralCategory => write!(f, "plural-category"),
            Self::NumberOrPluralCategory => write!(f, "number-or-plural-category"),
        }
    }
}

impl fmt::Display for FunctionOptionValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal => write!(f, "literal"),
            Self::Variable => write!(f, "variable"),
            Self::Either => write!(f, "literal or variable"),
        }
    }
}

/// Parse failure for [`FunctionManifest::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestParseError {
    /// 1-based source line.
    pub line: usize,
    /// 1-based source column.
    pub column: usize,
    /// Human-readable failure detail.
    pub detail: String,
}

impl ManifestParseError {
    fn new(line: usize, column: usize, detail: impl Into<String>) -> Self {
        Self {
            line,
            column,
            detail: detail.into(),
        }
    }

    fn from_toml(source: &str, error: toml::de::Error) -> Self {
        let (line, column) = error
            .span()
            .map_or((1, 1), |span| line_column(source, span.start));
        Self::new(line, column, error.to_string())
    }
}

impl fmt::Display for ManifestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid function manifest at {}:{} ({})",
            self.line, self.column, self.detail
        )
    }
}

impl std::error::Error for ManifestParseError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlManifest {
    #[serde(default)]
    functions: BTreeMap<String, TomlFunctionSchema>,
}

impl TomlManifest {
    fn into_manifest(self) -> FunctionManifest {
        let mut manifest = FunctionManifest::new();
        for (name, schema) in self.functions {
            manifest.insert(schema.into_schema(name));
        }
        manifest
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFunctionSchema {
    #[serde(default)]
    format: bool,
    #[serde(default)]
    select: bool,
    operand: Option<FunctionOperandKind>,
    selector_keys: Option<FunctionSelectorKeyKind>,
    #[serde(default)]
    allow_unknown_options: bool,
    display_name: Option<String>,
    category: Option<String>,
    doc: Option<String>,
    snippet: Option<String>,
    #[serde(default)]
    options: BTreeMap<String, TomlOptionSchema>,
}

impl TomlFunctionSchema {
    fn into_schema(self, name: String) -> FunctionSchema {
        let mut schema = FunctionSchema::new(name);
        schema.allows_format = self.format;
        schema.allows_select = self.select;
        schema.operand_kind = self.operand.unwrap_or(FunctionOperandKind::Any);
        schema.selector_keys = self.selector_keys.unwrap_or(FunctionSelectorKeyKind::Any);
        schema.allow_unknown_options = self.allow_unknown_options;
        schema.display_name = self.display_name;
        schema.category = self.category;
        schema.doc = self.doc;
        schema.snippet = self.snippet;
        schema.options = self
            .options
            .into_iter()
            .map(|(key, option)| {
                (
                    key.clone(),
                    FunctionOptionSchema {
                        key,
                        value_kind: option.value.unwrap_or(FunctionOptionValueKind::Either),
                        required: option.required,
                    },
                )
            })
            .collect();
        schema
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlOptionSchema {
    value: Option<FunctionOptionValueKind>,
    #[serde(default)]
    required: bool,
}

fn line_column(source: &str, byte_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (index, byte) in source.bytes().enumerate() {
        if index >= byte_index {
            break;
        }
        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::{
        FunctionManifest, FunctionOperandKind, FunctionOptionValueKind, FunctionSelectorKeyKind,
    };

    #[test]
    fn parses_manifest_toml() {
        let manifest = FunctionManifest::parse(
            r#"
[functions.number]
format = true
select = true
operand = "number"
selector_keys = "number-or-plural-category"

[functions.number.options.select]
value = "literal"

[functions.number.options.minimumFractionDigits]
value = "literal"
required = true

[functions."app:link"]
format = true
operand = "string"
display_name = "App Link"
category = "links"
doc = "Open an in-app link"
snippet = "{ $value :app:link href=$url }"

[functions."app:link".options.href]
value = "either"
"#,
        )
        .expect("manifest");

        let number = manifest.get("number").expect("number schema");
        assert!(number.allows_format);
        assert!(number.allows_select);
        assert_eq!(number.operand_kind, FunctionOperandKind::Number);
        assert_eq!(
            number.selector_keys,
            FunctionSelectorKeyKind::NumberOrPluralCategory
        );
        assert_eq!(
            number.option_schema("select").expect("option").value_kind,
            FunctionOptionValueKind::Literal
        );
        assert!(
            number
                .option_schema("minimumFractionDigits")
                .expect("option")
                .required
        );

        let link = manifest.get("app:link").expect("app:link schema");
        assert_eq!(link.operand_kind, FunctionOperandKind::String);
        assert_eq!(link.display_name.as_deref(), Some("App Link"));
        assert_eq!(link.category.as_deref(), Some("links"));
        assert_eq!(link.doc.as_deref(), Some("Open an in-app link"));
        assert_eq!(
            link.snippet.as_deref(),
            Some("{ $value :app:link href=$url }")
        );
        assert_eq!(
            link.option_schema("href").expect("href").value_kind,
            FunctionOptionValueKind::Either
        );
    }

    #[test]
    fn rejects_unknown_manifest_field() {
        let err = FunctionManifest::parse(
            r#"
[functions.number]
format = true
weird = true
"#,
        )
        .expect_err("must fail");
        assert_eq!(err.line, 4);
        assert!(err.detail.contains("unknown field"));
    }

    #[test]
    fn rejects_unknown_operand_kind() {
        let err = FunctionManifest::parse(
            r#"
[functions.number]
operand = "boolean"
"#,
        )
        .expect_err("must fail");
        assert_eq!(err.line, 3);
        assert!(err.detail.contains("unknown variant"));
    }
}
