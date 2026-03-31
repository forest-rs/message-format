// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use message_format::runtime::{Args, FormatError, FormatSink, Formatter, Host, MessageHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormatOutput {
    pub(crate) value: String,
    pub(crate) errors: Vec<FormatError>,
}

struct OutputStringSink<'a> {
    out: &'a mut String,
}

impl FormatSink for OutputStringSink<'_> {
    fn literal(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn expression(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn markup_open(&mut self, _name: &str, _options: &[message_format::runtime::FormatOption<'_>]) {
    }

    fn markup_close(
        &mut self,
        _name: &str,
        _options: &[message_format::runtime::FormatOption<'_>],
    ) {
    }
}

pub(crate) fn format<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message: MessageHandle,
    args: &dyn Args,
) -> Result<String, FormatError> {
    let mut out = String::new();
    let mut sink = OutputStringSink { out: &mut out };
    let _diagnostics = formatter.format_to(message, args, &mut sink)?;
    Ok(out)
}

pub(crate) fn format_by_id<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message_id: &str,
    args: &dyn Args,
) -> Result<String, FormatError> {
    let message = formatter.resolve(message_id)?;
    format(formatter, message, args)
}

pub(crate) fn format_with_diagnostics_by_id<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message_id: &str,
    args: &dyn Args,
) -> Result<FormatOutput, FormatError> {
    let message = formatter.resolve(message_id)?;
    let mut out = String::new();
    let mut sink = OutputStringSink { out: &mut out };
    let errors = formatter.format_to(message, args, &mut sink)?;
    Ok(FormatOutput { value: out, errors })
}

#[cfg(test)]
pub(crate) fn format_to_by_id<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message_id: &str,
    args: &dyn Args,
    sink: &mut dyn FormatSink,
) -> Result<Vec<FormatError>, FormatError> {
    let message = formatter.resolve(message_id)?;
    formatter.format_to(message, args, sink)
}
