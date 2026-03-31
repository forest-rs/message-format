// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{string::String, vec::Vec};

use crate::runtime;

/// Named argument buffer for facade formatting APIs.
///
/// Unlike [`runtime::MessageArgs`], this type is not bound to one concrete
/// runtime catalog. Names are resolved against the active catalog of the
/// formatter at format time, which keeps bundle-based locale lookup coherent.
///
/// Unknown names are ignored when resolved against a catalog. That matches the
/// facade's plain-string focus: extra arguments do not fail formatting, while
/// missing referenced arguments still surface through normal message fallback.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageArgs {
    values: Vec<(String, runtime::Value)>,
}

impl MessageArgs {
    /// Creates an empty named argument buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty named argument buffer with reserved capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    /// Inserts or replaces one named argument.
    ///
    /// Returns the previous value when the same name was already present.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<runtime::Value>,
    ) -> Option<runtime::Value> {
        let name = name.into();
        let value = value.into();
        if let Some((_, existing)) = self.values.iter_mut().find(|(key, _)| *key == name) {
            return Some(core::mem::replace(existing, value));
        }
        self.values.push((name, value));
        None
    }

    /// Appends one named argument and ignores any replaced value.
    pub fn push(&mut self, name: impl Into<String>, value: impl Into<runtime::Value>) {
        let _ = self.insert(name, value);
    }

    /// Removes all buffered arguments without releasing backing allocation.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Returns the buffered named values as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[(String, runtime::Value)] {
        self.values.as_slice()
    }

    pub(crate) fn resolve<'a>(&self, catalog: &'a runtime::Catalog) -> runtime::MessageArgs<'a> {
        let mut resolved = runtime::MessageArgs::with_capacity(catalog, self.values.len());
        for (name, value) in &self.values {
            if let Some(id) = catalog.string_id(name) {
                resolved.insert_id(id, value.clone());
            }
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::MessageArgs;
    use crate::runtime;

    #[test]
    fn insert_replaces_existing_named_value() {
        let mut args = MessageArgs::new();
        assert!(args.insert("name", "Ada").is_none());
        let replaced = args.insert("name", "Grace").expect("old value");
        assert_eq!(replaced, runtime::Value::Str(String::from("Ada")));
        assert_eq!(args.as_slice().len(), 1);
    }
}
