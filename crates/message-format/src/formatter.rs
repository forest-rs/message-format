// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::String;

use icu_locale_core::Locale;

use crate::{MessageArgs, options::LocalePolicy, runtime};

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

/// Reusable formatter bound to a loaded catalog and host behavior.
#[derive(Debug)]
pub struct MessageFormatter<'a> {
    catalog: &'a runtime::Catalog,
    #[cfg(feature = "icu4x")]
    inner: runtime::Formatter<'a, runtime::BuiltinHost>,
    #[cfg(not(feature = "icu4x"))]
    inner: runtime::Formatter<'a, runtime::NoopHost>,
}

impl<'a> MessageFormatter<'a> {
    #[cfg(feature = "icu4x")]
    pub(crate) fn new(
        catalog: &'a runtime::Catalog,
        locale: &Locale,
        policy: LocalePolicy,
    ) -> Result<Self, runtime::FormatError> {
        match policy {
            LocalePolicy::Exact => {
                let host = runtime::BuiltinHost::new(locale)?;
                Ok(Self {
                    catalog,
                    inner: runtime::Formatter::new(catalog, host)?,
                })
            }
            LocalePolicy::Lookup => {
                let mut last_err = None;
                for candidate in runtime::locale_fallback_candidates(locale) {
                    match runtime::BuiltinHost::new(&candidate) {
                        Ok(host) => {
                            return Ok(Self {
                                catalog,
                                inner: runtime::Formatter::new(catalog, host)?,
                            });
                        }
                        Err(err) => last_err = Some(err),
                    }
                }
                Err(last_err
                    .unwrap_or(runtime::FormatError::Trap(runtime::Trap::UnsupportedLocale)))
            }
        }
    }

    #[cfg(not(feature = "icu4x"))]
    pub(crate) fn new(
        catalog: &'a runtime::Catalog,
        _locale: &Locale,
        _policy: LocalePolicy,
    ) -> Result<Self, runtime::FormatError> {
        Ok(Self {
            catalog,
            inner: runtime::Formatter::new(catalog, runtime::NoopHost)?,
        })
    }

    /// Set the maximum number of VM instructions per format operation.
    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.inner.set_fuel(fuel);
    }

    /// Resolve a message id to a reusable handle.
    ///
    /// Reuse the returned handle across repeated formatting calls to avoid
    /// per-call message-id lookup.
    pub fn resolve(
        &self,
        message_id: &str,
    ) -> Result<runtime::MessageHandle, runtime::FormatError> {
        self.inner.resolve(message_id)
    }

    /// Format one message from a previously resolved handle.
    ///
    /// Recoverable diagnostics from fallback rendering are ignored in this
    /// convenience API. Markup is flattened away; use
    /// [`runtime::Formatter::format_to`] for structured output.
    pub fn format(
        &mut self,
        message: runtime::MessageHandle,
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
        message: runtime::MessageHandle,
        args: &MessageArgs,
        out: &mut String,
    ) -> Result<(), runtime::FormatError> {
        out.clear();
        let resolved = args.resolve(self.catalog);
        let mut sink = OutputStringSink { out };
        let _diagnostics = self.inner.format_to(message, &resolved, &mut sink)?;
        Ok(())
    }

    /// Format one message by id.
    ///
    /// Recoverable diagnostics from fallback rendering are ignored in this
    /// convenience API. Markup is flattened away; use
    /// [`runtime::Formatter::format_to`] for structured output.
    pub fn format_by_id(
        &mut self,
        message_id: &str,
        args: &MessageArgs,
    ) -> Result<String, runtime::FormatError> {
        let message = self.resolve(message_id)?;
        self.format(message, args)
    }
}
