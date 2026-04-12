// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// LINEBENDER LINT SET - lib.rs - v4
// See https://linebender.org/wiki/canonical-lints/
// These lints shouldn't apply to examples or tests.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
// These lints shouldn't apply to examples.
#![warn(clippy::print_stdout, clippy::print_stderr)]
// Targeting e.g. 32-bit means structs containing usize can give false positives for 64-bit.
#![cfg_attr(target_pointer_width = "64", warn(clippy::trivially_copy_pass_by_ref))]
// END LINEBENDER LINT SET
#![cfg_attr(docsrs, feature(doc_cfg))]
#![no_std]
#![doc = "Runtime for loading and executing compiled message-format catalogs."]
//!
//! # Quick Start
//!
//! Binary wire layout for catalogs is documented in `spec/catalog-binary-format.md`
//! at the workspace root.
//!
//! Runtime/compiler error contracts are summarized in `docs/error-model.md`
//! at the workspace root.
//! VM execution invariants are documented in `docs/runtime-vm-semantics.md`
//! at the workspace root.
//!
//! Catalogs are expected to come from the compiler or prebuilt assets. The
//! examples below assume you already loaded a compiled catalog.
//!
//! ```rust,no_run
//! use message_format_runtime::{
//!     Catalog, FormatError, FormatOption, FormatSink, Formatter, MessageArgs, NoopHost,
//! };
//!
//! struct StringSink<'a>(&'a mut String);
//!
//! impl message_format_runtime::FormatSink for StringSink<'_> {
//!     fn literal(&mut self, s: &str) {
//!         self.0.push_str(s);
//!     }
//!
//!     fn expression(&mut self, s: &str) {
//!         self.0.push_str(s);
//!     }
//!
//!     fn markup_open(
//!         &mut self,
//!         _name: &str,
//!         _options: &[FormatOption<'_>],
//!     ) {}
//!
//!     fn markup_close(
//!         &mut self,
//!         _name: &str,
//!         _options: &[FormatOption<'_>],
//!     ) {}
//! }
//!
//! # fn render(catalog: &Catalog) -> Result<String, FormatError> {
//! let mut formatter = Formatter::new(catalog, NoopHost)?;
//! let message = formatter.resolve("main")?;
//! let mut args = MessageArgs::new(catalog);
//! args.insert("name", "Ada")
//!     .expect("catalog defines `name`");
//! let mut out = String::new();
//! let mut sink = StringSink(&mut out);
//! let _errors = formatter.format_to(message, &args, &mut sink)?;
//! # Ok(out)
//! # }
//! ```
//!
//! # Structured Output and Markup
//!
//! [`Formatter::format_to`] is the advanced runtime entry point. It streams
//! four kinds of events into a [`FormatSink`]:
//!
//! - literal text copied directly from the pattern
//! - expression text produced by variables, literals, and function results
//! - markup open tags with resolved options
//! - markup close tags with resolved options
//!
//! Plain string-oriented helpers can ignore markup entirely, but sinks that
//! build rich output should preserve the `markup_open` / `markup_close` events.
//! Self-closing markup is represented as an immediate open event followed by a
//! close event with the same name and options.
//!
//! Markup option values arrive as already-resolved display text via
//! [`FormatOption`]. Option keys are interned catalog strings and option values
//! are resolved from literals or variables before delivery to the sink.
//!
//! Glossary:
//!
//! - sink: a caller-provided receiver for literal text, expression text, and markup events
//! - markup event: one `markup_open` or `markup_close` callback with resolved options
//! - resolved option: a key/value pair where the value is already formatted for display
//!
//! ```rust,no_run
//! use message_format_runtime::{
//!     Catalog, FormatError, FormatOption, FormatSink, Formatter, MessageArgs, NoopHost,
//! };
//!
//! #[derive(Debug, PartialEq, Eq)]
//! enum Event {
//!     Literal(String),
//!     Expression(String),
//!     Open(String, Vec<(String, String)>),
//!     Close(String, Vec<(String, String)>),
//! }
//!
//! #[derive(Default)]
//! struct CollectingSink {
//!     events: Vec<Event>,
//! }
//!
//! impl FormatSink for CollectingSink {
//!     fn literal(&mut self, s: &str) {
//!         self.events.push(Event::Literal(s.to_string()));
//!     }
//!
//!     fn expression(&mut self, s: &str) {
//!         self.events.push(Event::Expression(s.to_string()));
//!     }
//!
//!     fn markup_open(&mut self, name: &str, options: &[FormatOption<'_>]) {
//!         self.events.push(Event::Open(
//!             name.to_string(),
//!             options
//!                 .iter()
//!                 .map(|option| (option.key.to_string(), option.value.to_string()))
//!                 .collect(),
//!         ));
//!     }
//!
//!     fn markup_close(&mut self, name: &str, options: &[FormatOption<'_>]) {
//!         self.events.push(Event::Close(
//!             name.to_string(),
//!             options
//!                 .iter()
//!                 .map(|option| (option.key.to_string(), option.value.to_string()))
//!                 .collect(),
//!         ));
//!     }
//! }
//!
//! # fn collect(catalog: &Catalog) -> Result<Vec<Event>, FormatError> {
//! let mut formatter = Formatter::new(catalog, NoopHost)?;
//! let message = formatter.resolve("main")?;
//! let mut args = MessageArgs::new(catalog);
//! args.insert("url", "https://example.com")
//!     .expect("catalog defines `url`");
//! let mut sink = CollectingSink::default();
//! let errors = formatter.format_to(message, &args, &mut sink)?;
//! assert!(errors.is_empty());
//! # Ok(sink.events)
//! # }
//! ```
//!
//! # Error Model
//!
//! - [`CatalogError`] is returned when catalog bytes are invalid or unsupported.
//! - [`FormatError`] is returned while resolving/formatting messages.
//! - [`HostCallError`] constrains host callbacks to unknown-function failures or
//!   typed [`MessageFunctionError`] values.
//! - [`MessageFunctionError`] carries spec-shaped function error categories, with
//!   structured sub-enums for unsupported operations and implementation-defined
//!   host failures.
//! - [`FormatError::Decode`] wraps [`CatalogError`] values when decode errors
//!   surface through formatting APIs.
//!
//! # Schema Ownership
//!
//! The runtime owns the executable catalog schema in [`schema`]. The compiler
//! targets that shared schema when emitting binary catalogs, and the runtime
//! verifier/VM interpret the same schema when loading and executing them.

extern crate alloc;

#[cfg(feature = "icu4x")]
#[cfg_attr(docsrs, doc(cfg(feature = "icu4x")))]
pub use builtin::BuiltinHost;
#[cfg(feature = "icu4x")]
#[cfg_attr(docsrs, doc(cfg(feature = "icu4x")))]
pub use builtin::BuiltinHostCatalogIndex;
#[cfg(feature = "icu4x")]
#[cfg_attr(docsrs, doc(cfg(feature = "icu4x")))]
pub use builtin::locale_fallback_candidates;
pub use catalog::Catalog;
pub use error::{
    CatalogError, FormatError, HostCallError, ImplementationFailure, MessageFunctionError, Trap,
    UnsupportedOperation,
};
pub use formatter::{Formatter, MultiFormatter, MultiMessageHandle};
pub use schema::{FuncEntry, MessageEntry};
pub use value::{ArgNameError, Args, MessageArgs, StrId, Value};
pub use vm::{FormatOption, FormatSink, Host, HostFn, MessageHandle, NoopHost};

/// Catalog decoding and verification.
pub mod catalog;

/// Shared executable catalog schema used by both compiler and runtime.
pub mod schema;

/// Runtime errors.
mod error;

/// High-level formatting wrappers.
mod formatter;

/// Runtime value model and argument adapters.
mod value;

/// Bytecode execution engine.
pub mod vm;

/// ICU4X-backed built-in host functions.
#[cfg(feature = "icu4x")]
mod builtin;
