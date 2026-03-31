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
#![doc = "Compiler from message format source text to binary catalog bytes."]
//!
//! # Error Model
//!
//! The compiler reports failures as [`CompileError`]. Most parse/lowering
//! variants include line/column plus optional `expected`/`found` details.
//!
//! For a detailed cross-crate error model, see
//! `docs/error-model.md` at the workspace root.
//!
//! ## Diagnostic Rendering
//!
//! [`CompileError::render_with_source`] can render a snippet with a caret:
//!
//! ```rust
//! use message_format_compiler::compile_str;
//!
//! let err = compile_str("{ $name : }").expect_err("must fail");
//! let rendered = err.render_with_source("{ $name : }");
//! assert!(rendered.contains("error:"));
//! assert!(rendered.contains("^"));
//! ```
//!
//! ## Structured Inputs
//!
//! Non-text producers can build compiler-semantic messages directly and
//! compile them through [`CatalogBuilder`]:
//!
//! ```rust
//! use message_format_compiler::{CatalogBuilder, SourceKind, SourceSpan};
//! use message_format_compiler::semantic::{Message, SelectExpr, SelectorExpr, Part};
//!
//! let mut builder = CatalogBuilder::new();
//! let source_id = builder
//!     .register_source("generated", SourceKind::Generated)
//!     .expect("source id");
//! builder.add_message(
//!     Message::builder("main")
//!         .origin(SourceSpan {
//!             source_id,
//!             byte_start: 0,
//!             byte_end: 32,
//!             line: Some(1),
//!             column: Some(1),
//!         })
//!         .select(
//!             SelectExpr::builder(SelectorExpr::var("kind"))
//!                 .arm("formal", vec![Part::text("Good evening")])
//!                 .default(vec![Part::text("Hi")])
//!                 .build(),
//!         )
//!         .build(),
//! )
//! .expect("message");
//! let compiled = builder.compile().into_result().expect("compiled");
//! assert!(!compiled.bytes.is_empty());
//! ```
//!
//! This builder path is intentionally low-level. It exposes the compiler's
//! current internal semantic model, not a calmer end-user authoring format.
//! Prefer resource adapters when your input already lives in a container format
//! such as JSON, TOML, XLIFF, or similar.
//!
//! Use the builder path when:
//! - your input is already structured data generated in-process
//! - you need to attach compiler provenance directly
//! - you are comfortable targeting compiler-semantic parts explicitly
//!
//! Prefer resource adapters when:
//! - your input is a real container format such as JSON or TOML
//! - you want container-specific provenance preserved
//! - you want the compiler surface to stay calmer than `Message` / `Part`
//!
//! Resource/container formats sit above the core MF2 compiler. When you
//! already have named MF2 message bodies from some outer format, compile them
//! through [`ResourceInput`] instead of pretending that outer format is MF2:
//!
//! ```rust
//! use message_format_compiler::{CompileOptions, ResourceInput, compile_resources};
//!
//! let resources = ResourceInput::new("app.toml", message_format_compiler::SourceKind::Other("resource-toml".into()))
//!     .message("app.title", "Example App")
//!     .message("user.greeting", "Hello { $name }");
//! let compiled = compile_resources([resources], CompileOptions::default())
//!     .into_result()
//!     .expect("compiled");
//! assert!(!compiled.bytes.is_empty());
//! ```
//!
//! ## Function Manifests
//!
//! Function manifests let the compiler validate custom function names, call
//! sites, operands, and option shapes without linking executable host code.
//! They are TOML files consumed today by the compiler and CLI for validation.
//!
//! ```toml
//! [functions."app:link"]
//! format = true
//! operand = "string"
//!
//! [functions."app:link".options.href]
//! value = "either"
//! required = true
//! ```
//!
//! ## Executable Schema Ownership
//!
//! The compiler owns source parsing, semantic analysis, lowering, and catalog
//! emission. The executable catalog schema it targets lives in
//! [`message_format_runtime::schema`], which is also used by the runtime
//! verifier and VM.

pub use compile::{
    BuildError, BuildErrorContext, CatalogBuilder, CompileError, CompileInput, CompileOptions,
    CompileReport, CompiledCatalog, DiagnosticContext, DiagnosticSeverity, MessageSource,
    SourceMap, compile, compile_inputs, compile_inputs_with_manifest, compile_resources,
    compile_resources_with_manifest, compile_str, compile_with_manifest, escape_text,
};
pub use manifest::{
    FunctionManifest, FunctionOperandKind, FunctionOptionSchema, FunctionOptionValueKind,
    FunctionSchema, FunctionSelectorKeyKind, ManifestParseError,
};
pub use resource::{MessageResource, ResourceInput, ResourceSpan};
pub use semantic::{DetachedSourceSpan, SourceId, SourceInfo, SourceKind, SourceSpan};

mod compile;
mod manifest;
mod resource;

/// Semantic representation used between frontend analysis and bytecode lowering.
pub mod semantic;

/// Legacy alias for the compiler semantic representation.
pub mod ir;

mod syntax;
