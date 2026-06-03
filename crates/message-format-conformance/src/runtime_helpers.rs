// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use message_format::runtime::{Args, FormatError, Formatter, Host, MessageHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormatOutput {
    pub(crate) value: String,
    pub(crate) errors: Vec<FormatError>,
}

pub(crate) fn format<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message: MessageHandle,
    args: &dyn Args,
) -> Result<String, FormatError> {
    let mut out = String::new();
    let _diagnostics = formatter.format_to(message, args, &mut out)?;
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
    let errors = formatter.format_to(message, args, &mut out)?;
    Ok(FormatOutput { value: out, errors })
}

#[cfg(test)]
pub(crate) fn format_to_by_id<H: Host>(
    formatter: &mut Formatter<'_, H>,
    message_id: &str,
    args: &dyn Args,
    sink: &mut dyn message_format::runtime::FormatSink,
) -> Result<Vec<FormatError>, FormatError> {
    let message = formatter.resolve(message_id)?;
    formatter.format_to(message, args, sink)
}
