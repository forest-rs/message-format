// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(feature = "compile")]
use alloc::{boxed::Box, format, string::String};

#[cfg(not(feature = "icu4x"))]
use alloc::vec;
use alloc::vec::Vec;

use icu_locale_core::Locale;

use crate::{formatter::MessageFormatter, options::LocalePolicy, runtime};

#[cfg(feature = "icu4x")]
fn locale_candidates(locale: &Locale) -> Vec<Locale> {
    runtime::locale_fallback_candidates(locale)
}

#[cfg(not(feature = "icu4x"))]
fn locale_candidates(locale: &Locale) -> Vec<Locale> {
    vec![locale.clone()]
}

/// Loaded catalog with high-level formatter construction APIs.
#[derive(Debug, Clone)]
pub struct MessageCatalog {
    catalog: runtime::Catalog,
}

impl MessageCatalog {
    /// Decode a serialized catalog payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, runtime::CatalogError> {
        Ok(Self {
            catalog: runtime::Catalog::from_bytes(bytes)?,
        })
    }

    /// Access the underlying runtime catalog.
    #[must_use]
    pub fn as_runtime_catalog(&self) -> &runtime::Catalog {
        &self.catalog
    }

    /// Resolve a message id to a reusable handle.
    pub fn resolve(
        &self,
        message_id: &str,
    ) -> Result<runtime::MessageHandle, runtime::FormatError> {
        runtime::MessageHandle::from_catalog(&self.catalog, message_id)
    }

    /// Resolve one string-pool id for reuse across formatting calls.
    #[must_use]
    pub fn string_id(&self, value: &str) -> Option<runtime::StrId> {
        self.catalog.string_id(value)
    }

    /// Create a formatter bound to one locale for repeated formatting calls.
    ///
    /// Equivalent to calling [`Self::formatter_with_locale`] with
    /// [`LocalePolicy::Lookup`].
    pub fn formatter_for_locale(
        &self,
        locale: &Locale,
    ) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        self.formatter_with_locale(locale, LocalePolicy::Lookup)
    }

    /// Create a formatter bound to one locale with explicit fallback policy.
    ///
    /// With `icu4x`, `locale` is resolved according to `policy`.
    /// Without `icu4x`, `locale` is ignored.
    pub fn formatter_with_locale(
        &self,
        locale: &Locale,
        policy: LocalePolicy,
    ) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        MessageFormatter::new(&self.catalog, locale, policy)
    }
}

#[cfg(feature = "compile")]
#[cfg_attr(docsrs, doc(cfg(feature = "compile")))]
impl MessageCatalog {
    /// Compile one MF2 message source string and decode it into a loaded catalog.
    pub fn compile(
        source: &str,
        options: crate::compiler::CompileOptions,
    ) -> Result<Self, crate::compiler::CompileError> {
        let bytes = crate::compiler::compile(source, options)?;
        Self::from_compiler_bytes(&bytes)
    }

    /// Compile one MF2 message source string with a function manifest and decode it into a loaded catalog.
    pub fn compile_with_manifest(
        source: &str,
        options: crate::compiler::CompileOptions,
        manifest: &crate::compiler::FunctionManifest,
    ) -> Result<Self, crate::compiler::CompileError> {
        let bytes = crate::compiler::compile_with_manifest(source, options, manifest)?;
        Self::from_compiler_bytes(&bytes)
    }

    /// Compile one MF2 message source string with default options and decode it into a loaded catalog.
    pub fn compile_str(source: &str) -> Result<Self, crate::compiler::CompileError> {
        let bytes = crate::compiler::compile_str(source)?;
        Self::from_compiler_bytes(&bytes)
    }

    /// Compile multiple explicitly keyed MF2 inputs into one loaded catalog and return source-map metadata.
    pub fn compile_inputs<'a>(
        inputs: impl IntoIterator<Item = crate::compiler::CompileInput<'a>>,
        options: crate::compiler::CompileOptions,
    ) -> Result<(Self, crate::compiler::SourceMap), crate::compiler::BuildError> {
        let compiled = crate::compiler::compile_inputs(inputs, options)
            .into_result()
            .map_err(first_build_error)?;
        let catalog =
            Self::from_bytes(&compiled.bytes).map_err(|err| crate::compiler::BuildError {
                severity: crate::compiler::DiagnosticSeverity::Error,
                context: None,
                error: Box::new(crate::compiler::CompileError::InternalError {
                    detail: format!("compiler emitted invalid catalog: {err:?}"),
                }),
            })?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple resource/container inputs into one loaded catalog and return source-map metadata.
    pub fn compile_resources(
        inputs: impl IntoIterator<Item = crate::compiler::ResourceInput>,
        options: crate::compiler::CompileOptions,
    ) -> Result<(Self, crate::compiler::SourceMap), crate::compiler::BuildError> {
        let compiled = crate::compiler::compile_resources(inputs, options)
            .into_result()
            .map_err(first_build_error)?;
        let catalog =
            Self::from_bytes(&compiled.bytes).map_err(|err| crate::compiler::BuildError {
                severity: crate::compiler::DiagnosticSeverity::Error,
                context: None,
                error: Box::new(crate::compiler::CompileError::InternalError {
                    detail: format!("compiler emitted invalid catalog: {err:?}"),
                }),
            })?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple explicitly keyed MF2 inputs using a function manifest and return source-map metadata.
    pub fn compile_inputs_with_manifest<'a>(
        inputs: impl IntoIterator<Item = crate::compiler::CompileInput<'a>>,
        options: crate::compiler::CompileOptions,
        manifest: &crate::compiler::FunctionManifest,
    ) -> Result<(Self, crate::compiler::SourceMap), crate::compiler::BuildError> {
        let compiled = crate::compiler::compile_inputs_with_manifest(inputs, options, manifest)
            .into_result()
            .map_err(first_build_error)?;
        let catalog =
            Self::from_bytes(&compiled.bytes).map_err(|err| crate::compiler::BuildError {
                severity: crate::compiler::DiagnosticSeverity::Error,
                context: None,
                error: Box::new(crate::compiler::CompileError::InternalError {
                    detail: format!("compiler emitted invalid catalog: {err:?}"),
                }),
            })?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple resource/container inputs using a function manifest and return source-map metadata.
    pub fn compile_resources_with_manifest(
        inputs: impl IntoIterator<Item = crate::compiler::ResourceInput>,
        options: crate::compiler::CompileOptions,
        manifest: &crate::compiler::FunctionManifest,
    ) -> Result<(Self, crate::compiler::SourceMap), crate::compiler::BuildError> {
        let compiled = crate::compiler::compile_resources_with_manifest(inputs, options, manifest)
            .into_result()
            .map_err(first_build_error)?;
        let catalog =
            Self::from_bytes(&compiled.bytes).map_err(|err| crate::compiler::BuildError {
                severity: crate::compiler::DiagnosticSeverity::Error,
                context: None,
                error: Box::new(crate::compiler::CompileError::InternalError {
                    detail: format!("compiler emitted invalid catalog: {err:?}"),
                }),
            })?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile a file from disk and decode it into a loaded catalog.
    #[cfg(feature = "std")]
    pub fn compile_file(path: &std::path::Path) -> Result<Self, crate::compiler::CompileError> {
        let source = std::fs::read_to_string(path).map_err(|err| {
            crate::compiler::CompileError::IoError {
                path: path.to_path_buf(),
                source: err,
            }
        })?;
        Self::compile(&source, crate::compiler::CompileOptions::default())
    }

    fn from_compiler_bytes(bytes: &[u8]) -> Result<Self, crate::compiler::CompileError> {
        Self::from_bytes(bytes).map_err(|err| crate::compiler::CompileError::InternalError {
            detail: format!("compiler emitted invalid catalog: {err:?}"),
        })
    }
}

#[cfg(feature = "compile")]
fn first_build_error(errors: Vec<crate::compiler::BuildError>) -> crate::compiler::BuildError {
    errors
        .into_iter()
        .next()
        .unwrap_or(crate::compiler::BuildError {
            severity: crate::compiler::DiagnosticSeverity::Error,
            context: None,
            error: Box::new(crate::compiler::CompileError::InternalError {
                detail: String::from("compile report unexpectedly empty"),
            }),
        })
}

/// A catalog associated with one locale.
#[derive(Debug, Clone)]
pub struct LocalizedCatalog {
    /// Locale for this catalog.
    pub locale: Locale,
    /// Message catalog payload.
    pub catalog: MessageCatalog,
}

impl LocalizedCatalog {
    /// Construct a localized catalog pair.
    #[must_use]
    pub fn new(locale: Locale, catalog: MessageCatalog) -> Self {
        Self { locale, catalog }
    }
}

/// Collection of locale-scoped catalogs with fallback-aware formatter construction.
#[derive(Debug, Clone, Default)]
pub struct CatalogBundle {
    catalogs: Vec<LocalizedCatalog>,
}

impl CatalogBundle {
    /// Create an empty bundle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one localized catalog.
    pub fn insert(&mut self, locale: Locale, catalog: MessageCatalog) {
        if let Some(existing) = self
            .catalogs
            .iter_mut()
            .find(|entry| entry.locale == locale)
        {
            existing.catalog = catalog;
            return;
        }
        self.catalogs.push(LocalizedCatalog::new(locale, catalog));
    }

    /// Insert one pre-paired localized catalog.
    pub fn insert_localized(&mut self, localized: LocalizedCatalog) {
        self.insert(localized.locale, localized.catalog);
    }

    /// Create a formatter for locale by searching catalogs according to policy.
    pub fn formatter_with_locale(
        &self,
        locale: &Locale,
        policy: LocalePolicy,
    ) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        match policy {
            LocalePolicy::Exact => {
                let entry = self
                    .catalogs
                    .iter()
                    .find(|entry| &entry.locale == locale)
                    .ok_or(runtime::FormatError::Trap(
                        runtime::Trap::MissingLocaleCatalog,
                    ))?;
                entry
                    .catalog
                    .formatter_with_locale(locale, LocalePolicy::Exact)
            }
            LocalePolicy::Lookup => {
                for candidate in locale_candidates(locale) {
                    if let Some(entry) =
                        self.catalogs.iter().find(|entry| entry.locale == candidate)
                    {
                        return entry
                            .catalog
                            .formatter_with_locale(&candidate, LocalePolicy::Exact);
                    }
                }
                Err(runtime::FormatError::Trap(
                    runtime::Trap::MissingLocaleCatalog,
                ))
            }
        }
    }

    /// Create a formatter using lookup fallback policy.
    pub fn formatter_for_locale(
        &self,
        locale: &Locale,
    ) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        self.formatter_with_locale(locale, LocalePolicy::Lookup)
    }
}
