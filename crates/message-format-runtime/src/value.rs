// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime value and argument model.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::{error::Error, fmt};

use crate::Catalog;

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
    /// Reference to a catalog string-pool entry.
    StrRef(StrId),
    /// Reference to a literal slice in the catalog literal blob.
    LitRef {
        /// Offset into literal blob bytes.
        off: u32,
        /// Length in bytes.
        len: u32,
    },
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
/// use message_format_runtime::{Catalog, MessageArgs};
///
/// # fn build_args(catalog: &Catalog) -> Result<MessageArgs<'_>, message_format_runtime::ArgNameError> {
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
    use super::{ArgNameError, MessageArgs, Value};
    use crate::catalog::{MessageEntry, build_catalog};
    use alloc::string::ToString;

    fn test_catalog() -> crate::Catalog {
        crate::Catalog::from_bytes(&build_catalog(
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
}
