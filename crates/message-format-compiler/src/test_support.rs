// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test-only helpers shared across compiler-crate tests.

use message_format_runtime::{Catalog, FormatError, Formatter, Host, NoopHost};

use crate::compile::compile_str;

/// Test-only [`message_format_runtime::FormatSink`] that accumulates output into a `String`.
#[derive(Default)]
pub(crate) struct OutputStringSink {
    pub(crate) out: String,
}

impl message_format_runtime::FormatSink for OutputStringSink {
    fn literal(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn expression(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn markup_open(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}

    fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {
    }
}

/// Convenience extension trait to format a message by id into a `String`.
pub(crate) trait FormatterTestExt<H: Host> {
    /// Format `message_id` with `args` and return the accumulated output.
    fn format_by_id_for_test(
        &mut self,
        message_id: &str,
        args: &dyn message_format_runtime::Args,
    ) -> Result<String, FormatError>;
}

impl<H: Host> FormatterTestExt<H> for Formatter<'_, H> {
    fn format_by_id_for_test(
        &mut self,
        message_id: &str,
        args: &dyn message_format_runtime::Args,
    ) -> Result<String, FormatError> {
        let message = self.resolve(message_id)?;
        let mut sink = OutputStringSink::default();
        let _diagnostics = self.format_to(message, args, &mut sink)?;
        Ok(sink.out)
    }
}

/// Compile an MF2 simple-message source and format its `main` message with no args.
///
/// Panics on any compile, catalog-decode, or formatter failure. Intended for
/// whole-pipeline round-trip checks where the inputs are known-valid.
pub(crate) fn format_simple_main(source: &str) -> String {
    let bytes = compile_str(source).expect("compile_str");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog decode");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    formatter
        .format_by_id_for_test("main", &Vec::<(u32, message_format_runtime::Value)>::new())
        .expect("format_by_id_for_test")
}
