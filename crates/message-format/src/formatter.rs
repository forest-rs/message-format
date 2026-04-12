// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::String;

use icu_locale_core::Locale;

use crate::{MessageArgs, runtime};

struct OutputStringSink<'a> {
    out: &'a mut String,
}

impl runtime::FormatSink for OutputStringSink<'_> {
    fn literal(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn expression(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn markup_open(&mut self, _name: &str, _options: &[runtime::FormatOption<'_>]) {}

    fn markup_close(&mut self, _name: &str, _options: &[runtime::FormatOption<'_>]) {}
}

/// Reusable formatter that resolves messages across one or more catalogs.
///
/// When multiple catalogs are provided, messages are resolved by searching
/// catalogs in the order they were given. This enables message-level fallback:
/// if a message is missing from the primary catalog, it can be found in a
/// secondary one without duplicating messages at compile time.
///
/// Arguments are automatically resolved against the catalog that owns the
/// matched message, so string-pool ids stay consistent.
#[derive(Debug)]
pub struct MessageFormatter<'a> {
    #[cfg(feature = "icu4x")]
    inner: runtime::MultiFormatter<'a, runtime::BuiltinHost>,
    #[cfg(not(feature = "icu4x"))]
    inner: runtime::MultiFormatter<'a, runtime::NoopHost>,
}

impl<'a> MessageFormatter<'a> {
    #[cfg(feature = "icu4x")]
    pub(crate) fn new(
        catalogs: impl IntoIterator<Item = &'a runtime::Catalog>,
        candidates: impl IntoIterator<Item = Locale>,
    ) -> Result<Self, runtime::FormatError> {
        let mut last_err = None;
        for candidate in candidates {
            match runtime::BuiltinHost::new(&candidate) {
                Ok(host) => {
                    return Ok(Self {
                        inner: runtime::MultiFormatter::new(catalogs, host)?,
                    });
                }
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or(runtime::FormatError::Trap(runtime::Trap::UnsupportedLocale)))
    }

    #[cfg(not(feature = "icu4x"))]
    pub(crate) fn new(
        catalogs: impl IntoIterator<Item = &'a runtime::Catalog>,
        _candidates: impl IntoIterator<Item = Locale>,
    ) -> Result<Self, runtime::FormatError> {
        Ok(Self {
            inner: runtime::MultiFormatter::new(catalogs, runtime::NoopHost)?,
        })
    }

    /// Set the maximum number of VM instructions per format operation.
    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.inner.set_fuel(fuel);
    }

    /// Resolve a message id to a reusable handle.
    ///
    /// Searches catalogs in the order they were provided and returns a handle
    /// to the first catalog that contains the message. Reuse the returned
    /// handle across repeated formatting calls to avoid per-call lookup.
    pub fn resolve(
        &self,
        message_id: &str,
    ) -> Result<runtime::MultiMessageHandle, runtime::FormatError> {
        self.inner.resolve(message_id)
    }

    /// Format one message from a previously resolved handle.
    ///
    /// Arguments are resolved against the catalog that owns the matched
    /// message. Recoverable diagnostics from fallback rendering are ignored
    /// in this convenience API. Markup is flattened away; use
    /// [`runtime::MultiFormatter::format_to`] for structured output.
    pub fn format(
        &mut self,
        message: runtime::MultiMessageHandle,
        args: &MessageArgs,
    ) -> Result<String, runtime::FormatError> {
        let mut out = String::new();
        self.format_into(message, args, &mut out)?;
        Ok(out)
    }

    /// Format one message into a caller-provided output buffer.
    ///
    /// Recoverable diagnostics from fallback rendering are ignored in this
    /// convenience API. Use runtime-level sink APIs when diagnostics are needed.
    fn format_into(
        &mut self,
        message: runtime::MultiMessageHandle,
        args: &MessageArgs,
        out: &mut String,
    ) -> Result<(), runtime::FormatError> {
        out.clear();
        let catalog = self.inner.catalog_for(message)?;
        let resolved = args.resolve(catalog);
        let mut sink = OutputStringSink { out };
        let _diagnostics = self.inner.format_to(message, &resolved, &mut sink)?;
        Ok(())
    }

    /// Format one message by id.
    ///
    /// Recoverable diagnostics from fallback rendering are ignored in this
    /// convenience API. Markup is flattened away; use
    /// [`runtime::MultiFormatter::format_to`] for structured output.
    pub fn format_by_id(
        &mut self,
        message_id: &str,
        args: &MessageArgs,
    ) -> Result<String, runtime::FormatError> {
        let message = self.resolve(message_id)?;
        self.format(message, args)
    }
}
