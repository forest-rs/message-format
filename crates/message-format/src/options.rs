// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Locale resolution policy for formatter construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalePolicy {
    /// Use only the requested locale and return an error if unsupported.
    Exact,
    /// Try locale fallback before returning an error.
    ///
    /// Fallback order is: requested tag, progressively truncated subtags
    /// (`fr-CA-x-private` -> `fr-CA-x` -> `fr-CA` -> `fr`).
    #[default]
    Lookup,
}
