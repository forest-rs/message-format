// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime value and argument model.

use alloc::string::ToString;
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use core::{error::Error, fmt};

use crate::runtime::Catalog;

use fixed_decimal::Decimal;
use icu_experimental::dimension::currency::CurrencyType;

/// String-pool identifier.
pub type StrId = u32;

/// Runtime value model used by the VM.
///
/// `Value::Float` is used for locale-aware default interpolation paths.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// No value.
    Null,
    /// Boolean scalar.
    Bool(bool),
    /// Integer scalar.
    Int(i64),
    /// Floating-point scalar.
    Float(f64),
    /// Owned UTF-8 string.
    Str(String),
    /// String resolved by the `string` function, retaining direction metadata.
    String(ResolvedString),
    /// Reference to a catalog string-pool entry.
    StrRef(StrId),
    /// Fallback expression text from the catalog string pool.
    ///
    /// This preserves the recoverable-error state of a failed declaration so
    /// the value can render its fallback while remaining ineligible for
    /// selector matching.
    Fallback(StrId),
    /// Expression fallback produced by a function-resolution failure.
    ///
    /// This distinct provenance lets later selector projection avoid
    /// re-reporting the failed function as an operand error.
    FunctionFallback(StrId),
    /// Reference to a literal slice in the catalog literal blob.
    LitRef {
        /// Offset into literal blob bytes.
        off: u32,
        /// Length in bytes.
        len: u32,
    },
    /// A number resolved by a built-in numeric function.
    Number(ResolvedNumber),
    /// A resolved formatter value that retains its semantic source for a
    /// subsequent annotation while exposing its formatted presentation.
    Formatted(Box<ResolvedFormatted>),
    /// A value resolved by the test-only `test:select` function.
    ///
    /// The private payload preserves the function's selected precision across
    /// aliases and reannotations. Raw strings continue to follow ordinary
    /// string/numeric conversion rules.
    ResolvedSelect(Box<ResolvedSelect>),
}

/// Output of a non-selecting built-in formatter with its source value intact.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFormatted {
    pub(crate) source: Value,
    pub(crate) formatted: String,
    pub(crate) kind: super::vm::FormattedValueKind,
    pub(crate) selection: Option<ResolvedNumber>,
    pub(crate) currency: Option<ResolvedCurrencyOptions>,
}

/// Currency options retained across annotations of a resolved currency value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedCurrencyOptions {
    pub(crate) code: CurrencyType,
    pub(crate) display: CurrencyDisplay,
    pub(crate) sign: CurrencySign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrencyDisplay {
    Symbol,
    NarrowSymbol,
    Code,
    Name,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrencySign {
    Standard,
    Accounting,
}

impl ResolvedFormatted {
    pub(crate) fn new(source: Value, formatted: String) -> Self {
        Self {
            source,
            formatted,
            kind: super::vm::FormattedValueKind::String,
            selection: None,
            currency: None,
        }
    }

    pub(crate) fn number(source: Value, formatted: String) -> Self {
        Self {
            source,
            formatted,
            kind: super::vm::FormattedValueKind::Number,
            selection: None,
            currency: None,
        }
    }

    pub(crate) fn selectable(source: Value, formatted: String, selection: ResolvedNumber) -> Self {
        Self {
            source,
            formatted,
            kind: super::vm::FormattedValueKind::Number,
            selection: Some(selection),
            currency: None,
        }
    }

    pub(crate) fn currency(
        source: Value,
        formatted: String,
        currency: ResolvedCurrencyOptions,
    ) -> Self {
        Self {
            source,
            formatted,
            kind: super::vm::FormattedValueKind::Number,
            selection: None,
            currency: Some(currency),
        }
    }

    /// Return the formatted presentation used for interpolation.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.formatted
    }
}

impl Value {
    pub(crate) fn is_fallback(&self) -> bool {
        matches!(self, Self::Fallback(_) | Self::FunctionFallback(_))
    }
}

/// String payload resolved by the `string` function.
#[derive(Debug, Clone)]
pub struct ResolvedString {
    /// Raw resolved text without bidi isolation controls.
    text: ResolvedStringText,
    /// Direction requested by the string function.
    pub(crate) direction: StringDirection,
}

const INLINE_STRING_CAPACITY: usize = 24;

#[derive(Debug, Clone)]
enum ResolvedStringText {
    Inline {
        len: u8,
        bytes: [u8; INLINE_STRING_CAPACITY],
    },
    Integer {
        value: i64,
        len: u8,
        bytes: [u8; 20],
    },
    Heap(Box<str>),
}

/// Direction metadata retained on a resolved string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringDirection {
    /// No direction was requested by the string function.
    Unspecified,
    /// Automatic direction selection.
    Auto,
    /// Left-to-right isolation.
    Ltr,
    /// Right-to-left isolation.
    Rtl,
}

impl ResolvedString {
    pub(crate) fn plain_borrowed(text: &str) -> Self {
        Self::from_borrowed(text, StringDirection::Unspecified)
    }

    pub(crate) fn plain_owned(text: String) -> Self {
        Self::from_owned(text, StringDirection::Unspecified)
    }

    pub(crate) fn from_borrowed(text: &str, direction: StringDirection) -> Self {
        let text = if text.len() <= INLINE_STRING_CAPACITY {
            let mut bytes = [0; INLINE_STRING_CAPACITY];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            ResolvedStringText::Inline {
                len: u8::try_from(text.len()).expect("inline string length fits in u8"),
                bytes,
            }
        } else {
            ResolvedStringText::Heap(text.into())
        };
        Self { text, direction }
    }

    pub(crate) fn from_owned(text: String, direction: StringDirection) -> Self {
        if text.len() <= INLINE_STRING_CAPACITY {
            return Self::from_borrowed(&text, direction);
        }
        Self {
            text: ResolvedStringText::Heap(text.into_boxed_str()),
            direction,
        }
    }

    pub(crate) fn from_integer(text: &str, value: i64, direction: StringDirection) -> Self {
        let mut bytes = [0; 20];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Self {
            text: ResolvedStringText::Integer {
                value,
                len: u8::try_from(text.len()).expect("integer text length fits in u8"),
                bytes,
            },
            direction,
        }
    }

    pub(crate) fn integer_hint(&self) -> Option<i64> {
        match self.text {
            ResolvedStringText::Integer { value, .. } => Some(value),
            ResolvedStringText::Inline { .. } | ResolvedStringText::Heap(_) => None,
        }
    }

    /// Return the raw resolved text without direction isolation controls.
    #[must_use]
    pub fn text(&self) -> &str {
        match &self.text {
            ResolvedStringText::Inline { len, bytes } => {
                core::str::from_utf8(&bytes[..usize::from(*len)])
                    .expect("inline resolved strings originate from UTF-8")
            }
            ResolvedStringText::Integer { len, bytes, .. } => {
                core::str::from_utf8(&bytes[..usize::from(*len)])
                    .expect("resolved integer strings contain ASCII")
            }
            ResolvedStringText::Heap(text) => text,
        }
    }
}

impl PartialEq for ResolvedString {
    fn eq(&self, other: &Self) -> bool {
        self.direction == other.direction && self.text() == other.text()
    }
}

impl Eq for ResolvedString {}

/// Value produced by the test-only `test:select` function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSelect {
    text: String,
}

impl ResolvedSelect {
    pub(crate) fn new(text: String) -> Self {
        Self { text }
    }

    /// Return the resolved selector text without applying another function.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A numeric value resolved by a built-in numeric function.
//
// The fields stay private so hosts cannot accidentally manufacture a value
// whose options and numeric payload disagree. The runtime uses this value to
// carry an exact integer or decimal through subsequent annotations.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNumber {
    pub(crate) value: NumberValue,
    pub(crate) format: NumberFormatOptions,
    pub(crate) selection: NumberSelection,
    /// Whether a `select` option was explicitly resolved for this value.
    pub(crate) has_explicit_select: bool,
}

/// Exact numeric payload retained by [`ResolvedNumber`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NumberValue {
    /// Exact signed integer payload.
    Integer(i64),
    /// Exact finite decimal payload.
    Decimal(Box<Decimal>),
    /// A floating-point non-finite value retained for compatibility with the
    /// existing runtime rendering behavior.
    NonFinite(f64),
}

/// Parsed options needed to render a resolved number. Keeping these values
/// parsed makes default interpolation infallible after a function call has
/// validated the merged option set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberFormatOptions {
    pub(crate) minimum_fraction_digits: Option<u8>,
    pub(crate) maximum_fraction_digits: Option<u8>,
    pub(crate) minimum_significant_digits: Option<u8>,
    pub(crate) maximum_significant_digits: Option<u8>,
    pub(crate) minimum_integer_digits: Option<u8>,
    pub(crate) sign_display: NumberSignDisplay,
    pub(crate) notation_scientific: bool,
    pub(crate) grouping: NumberGrouping,
}

impl NumberFormatOptions {
    pub(crate) const DEFAULT: Self = Self {
        minimum_fraction_digits: None,
        maximum_fraction_digits: None,
        minimum_significant_digits: None,
        maximum_significant_digits: None,
        minimum_integer_digits: None,
        sign_display: NumberSignDisplay::Auto,
        notation_scientific: false,
        grouping: NumberGrouping::Auto,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberSignDisplay {
    Auto,
    Always,
    ExceptZero,
    Negative,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberGrouping {
    Auto,
    Always,
    Never,
    Min2,
}

/// Selection provenance retained with a resolved number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberSelection {
    /// No selection annotation was involved.
    None,
    /// The value came from a plural selection annotation.
    Plural,
    /// The value came from an ordinal selection annotation.
    Ordinal,
    /// The value came from an exact selection annotation.
    Exact,
    /// A variable selection annotation reported `BadOption` and cannot be
    /// used to match a later selection.
    Invalid,
}

impl ResolvedNumber {
    pub(crate) fn new(
        value: NumberValue,
        format: NumberFormatOptions,
        selection: NumberSelection,
        has_explicit_select: bool,
    ) -> Self {
        Self {
            value,
            format,
            selection,
            has_explicit_select,
        }
    }

    /// Return the exact finite numeric value as an ASCII decimal string.
    ///
    /// The returned text contains no locale formatting, grouping, or
    /// annotation options. Hosts can use it when handling a [`Value::Number`]
    /// operand without depending on the runtime's private numeric payload.
    #[must_use]
    pub fn text(&self) -> String {
        match &self.value {
            NumberValue::Integer(value) => value.to_string(),
            NumberValue::Decimal(value) => value.to_string(),
            NumberValue::NonFinite(value) => value.to_string(),
        }
    }
}

/// Error returned when building [`MessageArgs`] with a name that is not interned
/// in the catalog string pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgNameError {
    name: String,
}

impl ArgNameError {
    /// Returns the unknown argument name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ArgNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown argument name {}", self.name)
    }
}

impl Error for ArgNameError {}

/// Convenience argument buffer that resolves `&str` names through a borrowed
/// catalog and stores the resulting [`StrId`] pairs for reuse.
///
/// This is an ergonomic layer on top of the runtime's id-keyed [`Args`] model.
/// Name lookup happens when you call [`MessageArgs::insert`] or
/// [`MessageArgs::push`], not during formatting.
///
/// ```rust,no_run
/// use message_format::runtime::{Catalog, MessageArgs};
///
/// # fn build_args(catalog: &Catalog) -> Result<MessageArgs<'_>, message_format::runtime::ArgNameError> {
/// let mut args = MessageArgs::new(catalog);
/// args.insert("name", "Ada")?;
/// args.insert("count", 3_i64)?;
/// # Ok(args)
/// # }
/// ```
#[derive(Debug)]
pub struct MessageArgs<'a> {
    catalog: &'a Catalog,
    values: Vec<(StrId, Value)>,
}

impl<'a> MessageArgs<'a> {
    /// Creates an empty argument buffer bound to one catalog.
    pub fn new(catalog: &'a Catalog) -> Self {
        Self {
            catalog,
            values: Vec::new(),
        }
    }

    /// Creates an empty argument buffer with reserved capacity.
    pub fn with_capacity(catalog: &'a Catalog, capacity: usize) -> Self {
        Self {
            catalog,
            values: Vec::with_capacity(capacity),
        }
    }

    /// Inserts or replaces one argument by name.
    ///
    /// Returns the previous value when the same argument name was already set.
    pub fn insert(
        &mut self,
        name: &str,
        value: impl Into<Value>,
    ) -> Result<Option<Value>, ArgNameError> {
        let key = self.lookup(name)?;
        Ok(self.insert_id(key, value.into()))
    }

    /// Appends one argument by name and ignores any replaced value.
    pub fn push(&mut self, name: &str, value: impl Into<Value>) -> Result<(), ArgNameError> {
        let _ = self.insert(name, value)?;
        Ok(())
    }

    /// Inserts or replaces one argument by a previously resolved string id.
    pub fn insert_id(&mut self, key: StrId, value: Value) -> Option<Value> {
        if let Some((_, existing)) = self.values.iter_mut().find(|(id, _)| *id == key) {
            return Some(core::mem::replace(existing, value));
        }
        self.values.push((key, value));
        None
    }

    /// Removes all buffered arguments without releasing the backing allocation.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Returns the buffered `(StrId, Value)` pairs as a slice.
    pub fn as_slice(&self) -> &[(StrId, Value)] {
        self.values.as_slice()
    }

    /// Consumes the buffer and returns the owned `(StrId, Value)` pairs.
    pub fn into_vec(self) -> Vec<(StrId, Value)> {
        self.values
    }

    fn lookup(&self, name: &str) -> Result<StrId, ArgNameError> {
        self.catalog
            .string_id(name)
            .ok_or_else(|| ArgNameError { name: name.into() })
    }
}

/// Argument lookup interface keyed by catalog string-pool id.
///
/// Resolve argument names to [`StrId`] once from a [`Catalog`]
/// and reuse those ids across formatting calls.
pub trait Args {
    /// Returns a borrowed value for a given argument id.
    fn get_ref(&self, key: StrId) -> Option<&Value>;
}

impl Args for [(StrId, Value)] {
    fn get_ref(&self, key: StrId) -> Option<&Value> {
        self.iter()
            .find_map(|(id, value)| (*id == key).then_some(value))
    }
}

impl Args for Vec<(StrId, Value)> {
    fn get_ref(&self, key: StrId) -> Option<&Value> {
        Args::get_ref(self.as_slice(), key)
    }
}

impl Args for BTreeMap<StrId, Value> {
    fn get_ref(&self, key: StrId) -> Option<&Value> {
        self.get(&key)
    }
}

impl<const N: usize> Args for [(StrId, Value); N] {
    fn get_ref(&self, key: StrId) -> Option<&Value> {
        Args::get_ref(self.as_slice(), key)
    }
}

impl Args for MessageArgs<'_> {
    fn get_ref(&self, key: StrId) -> Option<&Value> {
        Args::get_ref(self.values.as_slice(), key)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        // User-provided strings are still owned in the runtime input model.
        // Borrowed caller text would require a separate input value view type.
        Self::Str(value.into())
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

#[cfg(test)]
mod tests {
    use super::StringDirection;
    use super::{ArgNameError, MessageArgs, ResolvedString, ResolvedStringText, Value};
    use crate::runtime::catalog::{MessageEntry, build_catalog};
    use alloc::{format, string::ToString};

    fn test_catalog() -> crate::runtime::Catalog {
        crate::runtime::Catalog::from_bytes(&build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &[0x20, 0, 0, 0, 0, 0],
        ))
        .expect("catalog")
    }

    #[test]
    fn message_args_resolves_and_replaces_values() {
        let catalog = test_catalog();
        let mut args = MessageArgs::new(&catalog);

        assert!(args.insert("main", "hello").expect("insert").is_none());
        let replaced = args
            .insert("main", "bye")
            .expect("replace")
            .expect("old value");

        assert_eq!(replaced, Value::Str("hello".to_string()));
        assert_eq!(args.as_slice().len(), 1);
    }

    #[test]
    fn message_args_reports_unknown_names() {
        let catalog = test_catalog();
        let mut args = MessageArgs::new(&catalog);

        let err = args.insert("missing", Value::Null).unwrap_err();
        assert_eq!(
            err,
            ArgNameError {
                name: "missing".to_string()
            }
        );
        assert_eq!(err.name(), "missing");
    }

    #[test]
    fn integer_string_provenance_does_not_affect_equality() {
        for value in [i64::MIN, i64::MAX] {
            let text = value.to_string();
            let hinted = ResolvedString::from_integer(&text, value, StringDirection::Unspecified);
            let ordinary = ResolvedString::from_borrowed(&text, StringDirection::Unspecified);
            assert_eq!(hinted, ordinary);
            assert_eq!(hinted.integer_hint(), Some(value));
        }
    }

    #[test]
    fn resolved_string_inline_boundary_counts_utf8_bytes() {
        let inline = "é".repeat(12);
        let heap = format!("{inline}a");
        let inline = ResolvedString::plain_borrowed(&inline);
        let heap = ResolvedString::plain_borrowed(&heap);

        assert!(matches!(inline.text, ResolvedStringText::Inline { .. }));
        assert!(matches!(heap.text, ResolvedStringText::Heap(_)));
        assert_eq!(inline.text(), "éééééééééééé");
        assert_eq!(heap.text(), "ééééééééééééa");
    }
}
