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
#![doc = "Facade crate for message-format runtime and optional compiler APIs."]

//! # Recommended Flow
//!
//! ```rust
//! # #[cfg(all(feature = "compile", feature = "icu4x"))]
//! # {
//! use message_format::{Locale, MessageArgs, MessageCatalog, compiler::CompileOptions};
//!
//! let source = "Hello { $name }!";
//! let catalog = MessageCatalog::compile(source, CompileOptions::default()).unwrap();
//! let locale: Locale = "en-US".parse().unwrap();
//! let mut formatter = catalog.formatter_for_locale(&locale).unwrap();
//!
//! let mut args = MessageArgs::new();
//! args.insert("name", "World");
//! assert_eq!(formatter.format_by_id("main", &args).unwrap(), "Hello World!");
//! # }
//! ```
//!
//! # Advanced Flow
//!
//! ```rust
//! # #[cfg(all(feature = "compile", feature = "icu4x"))]
//! # {
//! use message_format::{CatalogBundle, Locale, LocalePolicy, MessageArgs, MessageCatalog, compiler::CompileOptions};
//!
//! let mut bundle = CatalogBundle::new();
//! let fr: Locale = "fr".parse().unwrap();
//! let en: Locale = "en".parse().unwrap();
//! let fr_catalog = MessageCatalog::compile("Salut { $name }", CompileOptions::default()).unwrap();
//! let en_catalog = MessageCatalog::compile("Hello { $name }", CompileOptions::default()).unwrap();
//! bundle.insert(fr.clone(), fr_catalog);
//! bundle.insert(en.clone(), en_catalog);
//!
//! let requested: Locale = "fr-CA".parse().unwrap();
//! let mut formatter = bundle.formatter_with_locale(&requested, LocalePolicy::Lookup).unwrap();
//! let mut args = MessageArgs::new();
//! args.insert("name", "Ada");
//! assert_eq!(formatter.format_by_id("main", &args).unwrap(), "Salut Ada");
//! # }
//! ```
//!
//! # Rich Output
//!
//! The facade APIs optimize for plain string formatting. They do not expose the
//! runtime sink interface directly, and markup is intentionally flattened away
//! in string output.
//!
//! When you need structured output, resolved markup options, or recoverable
//! diagnostics from fallback rendering, drop down to the runtime APIs via
//! [`MessageCatalog::as_runtime_catalog`] and use
//! [`runtime::Formatter::format_to`].

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub use icu_locale_core::Locale;
pub use message_format_runtime as runtime;

#[cfg(feature = "compile")]
#[cfg_attr(docsrs, doc(cfg(feature = "compile")))]
pub use message_format_compiler as compiler;

mod args;
mod catalog;
mod formatter;
mod options;

pub use args::MessageArgs;
pub use catalog::{CatalogBundle, LocalizedCatalog, MessageCatalog};
pub use formatter::MessageFormatter;
pub use options::LocalePolicy;

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "compile", feature = "icu4x"))]
    use super::runtime::{FormatError, Trap};
    #[cfg(any(
        all(feature = "compile", feature = "icu4x"),
        all(feature = "compile", feature = "std")
    ))]
    use super::*;
    #[cfg(all(feature = "compile", feature = "std"))]
    use alloc::format;
    #[cfg(all(feature = "compile", feature = "icu4x"))]
    use alloc::string::String;
    #[cfg(all(feature = "compile", feature = "std"))]
    use core::sync::atomic::{AtomicU32, Ordering};

    #[cfg(all(feature = "compile", feature = "std"))]
    static TEMP_FILE_COUNTER: AtomicU32 = AtomicU32::new(0);

    #[cfg(all(feature = "compile", feature = "std"))]
    fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
        let mut path = std::path::PathBuf::from(".");
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!("{prefix}_{counter}.mf2"));
        path
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    fn locale(tag: &str) -> Locale {
        tag.parse::<Locale>().expect("locale")
    }

    #[cfg(all(feature = "compile", feature = "std", feature = "icu4x"))]
    #[test]
    fn compile_entry_points_preserve_simple_whitespace() {
        let source = "  hello  ";
        let catalog_direct =
            MessageCatalog::compile(source, compiler::CompileOptions::default()).expect("compile");
        let compiled_str = compiler::compile_str(source).expect("compile_str");
        let catalog_str = MessageCatalog::from_bytes(&compiled_str).expect("from bytes");

        let path = unique_temp_path("mf2_whitespace");
        std::fs::write(&path, source).expect("write temp source");
        let catalog_file = MessageCatalog::compile_file(&path).expect("compile_file");
        std::fs::remove_file(&path).expect("remove temp source");

        let mut fmt_direct = catalog_direct
            .formatter_for_locale(&locale("en-US"))
            .expect("fmt");
        let mut fmt_str = catalog_str
            .formatter_for_locale(&locale("en-US"))
            .expect("fmt");
        let mut fmt_file = catalog_file
            .formatter_for_locale(&locale("en-US"))
            .expect("fmt");
        let args = MessageArgs::new();
        let out_direct = fmt_direct
            .format_by_id("main", &args)
            .expect("format direct");
        let out_str = fmt_str
            .format_by_id("main", &args)
            .expect("format compile_str");
        let out_file = fmt_file.format_by_id("main", &args).expect("format file");

        assert_eq!(out_direct, "  hello  ");
        assert_eq!(out_str, "  hello  ");
        assert_eq!(out_file, "  hello  ");
    }

    #[cfg(all(feature = "compile", feature = "std"))]
    #[test]
    fn compile_file_missing_path_returns_io_error() {
        let path = unique_temp_path("mf2_missing_compile_file_test_should_not_exist");
        let _ = std::fs::remove_file(&path);
        let err = MessageCatalog::compile_file(&path).unwrap_err();
        match err {
            compiler::CompileError::IoError { path: got, source } => {
                assert_eq!(got, path);
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected IoError, got: {other:?}"),
        }
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn compile_resources_merges_named_message_bodies() {
        let (catalog, source_map) = MessageCatalog::compile_resources(
            [compiler::ResourceInput::new(
                "app.toml",
                compiler::SourceKind::Other(String::from("resource-toml")),
            )
            .message("hello", "Hello")
            .message("bye", "Bye")],
            compiler::CompileOptions::default(),
        )
        .expect("compile");

        assert_eq!(source_map.sources.len(), 1);
        assert!(catalog.resolve("hello").is_ok());
        assert!(catalog.resolve("bye").is_ok());
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn compile_with_manifest_validates_custom_functions() {
        let mut manifest = compiler::FunctionManifest::new();
        manifest.insert(compiler::FunctionSchema::new("custom:format").allow_format());

        let err = MessageCatalog::compile_with_manifest(
            "{ $value :custom:missing }",
            compiler::CompileOptions::default(),
            &manifest,
        )
        .expect_err("must fail");

        match err {
            compiler::CompileError::UnknownFunction { function, .. } => {
                assert_eq!(function, "custom:missing");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn bundle_lookup_falls_back_to_parent_locale_catalog() {
        let mut bundle = CatalogBundle::new();
        bundle.insert(
            locale("fr"),
            MessageCatalog::compile_str("Salut { $name }").expect("compile fr"),
        );
        bundle.insert(
            locale("en"),
            MessageCatalog::compile_str("Hello { $name }").expect("compile en"),
        );

        let mut formatter = bundle
            .formatter_with_locale(&locale("fr-CA"), LocalePolicy::Lookup)
            .expect("lookup formatter");

        let mut args = MessageArgs::new();
        args.insert("name", "Ada");
        assert_eq!(
            formatter.format_by_id("main", &args).expect("format"),
            "Salut Ada"
        );
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn bundle_exact_uses_requested_locale_catalog() {
        let mut bundle = CatalogBundle::new();
        let en_catalog = MessageCatalog::compile_str("Hello").expect("compile en");
        let fr_catalog = MessageCatalog::compile_str("Bonjour").expect("compile fr");
        bundle.insert(locale("en"), en_catalog);
        bundle.insert(locale("fr"), fr_catalog);

        let mut formatter = bundle
            .formatter_with_locale(&locale("fr"), LocalePolicy::Exact)
            .expect("exact formatter");
        let args = MessageArgs::new();
        assert_eq!(
            formatter.format_by_id("main", &args).expect("format"),
            "Bonjour"
        );
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn bundle_formatter_resolves_named_args_against_active_catalog() {
        let en_catalog = MessageCatalog::compile_str("Hello { $name }").expect("compile en");
        let fr_catalog =
            MessageCatalog::compile_str("Salut { $given } { $name }").expect("compile fr");

        let mut bundle = CatalogBundle::new();
        bundle.insert(locale("en"), en_catalog);
        bundle.insert(locale("fr"), fr_catalog);

        let mut formatter = bundle
            .formatter_with_locale(&locale("en-AU"), LocalePolicy::Lookup)
            .expect("lookup formatter");

        let mut args = MessageArgs::new();
        args.insert("name", "Ada");

        assert_eq!(
            formatter.format_by_id("main", &args).expect("format"),
            "Hello Ada"
        );
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn empty_bundle_reports_missing_locale_catalog() {
        let bundle = CatalogBundle::new();
        let err = bundle
            .formatter_with_locale(&locale("en"), LocalePolicy::Lookup)
            .expect_err("must fail");
        assert_eq!(err, FormatError::Trap(Trap::MissingLocaleCatalog));
    }

    #[cfg(all(feature = "compile", feature = "icu4x"))]
    #[test]
    fn bundle_lookup_reports_missing_locale_when_no_catalog_matches() {
        let mut bundle = CatalogBundle::new();
        bundle.insert(
            locale("fr"),
            MessageCatalog::compile_str("Bonjour").expect("compile fr"),
        );

        let err = bundle
            .formatter_with_locale(&locale("en-US"), LocalePolicy::Lookup)
            .expect_err("must fail");
        assert_eq!(err, FormatError::Trap(Trap::MissingLocaleCatalog));
    }
}
