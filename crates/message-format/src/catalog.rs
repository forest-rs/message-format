// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(feature = "compile")]
use alloc::{boxed::Box, format, string::String};

use alloc::vec;
use alloc::vec::Vec;

use icu_locale_core::Locale;

use crate::{formatter::MessageFormatter, runtime};

#[cfg(feature = "icu4x")]
pub(crate) fn locale_candidates(locale: &Locale) -> Vec<Locale> {
    runtime::locale_fallback_candidates(locale)
}

#[cfg(not(feature = "icu4x"))]
pub(crate) fn locale_candidates(locale: &Locale) -> Vec<Locale> {
    if locale.id.is_unknown() {
        vec![locale.clone()]
    } else {
        vec![locale.clone(), Locale::UNKNOWN]
    }
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

    /// Create a single-catalog formatter bound to one locale.
    ///
    /// Uses CLDR-aware locale fallback to find the best available host locale.
    /// For message-level fallback across multiple catalogs, use
    /// [`CatalogBundle::formatter`] instead.
    pub fn formatter_for_locale(
        &self,
        locale: &Locale,
    ) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        let candidates = locale_candidates(locale);
        MessageFormatter::new(core::iter::once(&self.catalog), &candidates)
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

/// Immutable collection of catalogs pre-sorted in locale fallback order.
///
/// Accepts a set of [`LocalizedCatalog`]s and a target locale at construction,
/// immediately filtering and ordering catalogs by the CLDR fallback chain.
/// Messages are resolved by searching catalogs in order, so a message missing
/// from a more-specific catalog can still be found in a less-specific one.
#[derive(Debug, Clone)]
pub struct CatalogBundle {
    catalogs: Vec<runtime::Catalog>,
    /// Formatting-locale candidates, independent of catalog locales
    candidates: Vec<Locale>,
}

impl CatalogBundle {
    /// Create a bundle targeting `locale` from the given catalogs.
    ///
    /// Computes the CLDR fallback chain for the requested locale and retains
    /// only catalogs whose locale appears in that chain, ordered from most
    /// specific to least. Returns an error if no catalog matches any
    /// candidate in the fallback chain.
    pub fn new(
        catalogs: impl IntoIterator<Item = LocalizedCatalog>,
        locale: &Locale,
    ) -> Result<Self, runtime::FormatError> {
        let candidates = locale_candidates(locale);
        let mut slots: Vec<Option<runtime::Catalog>> = vec![None; candidates.len()];
        for lc in catalogs {
            if let Some(pos) = candidates.iter().position(|c| *c == lc.locale) {
                slots[pos] = Some(lc.catalog.catalog);
            }
        }
        let catalogs: Vec<runtime::Catalog> = slots.into_iter().flatten().collect();
        if catalogs.is_empty() {
            return Err(runtime::FormatError::Trap(
                runtime::Trap::MissingLocaleCatalog,
            ));
        }
        Ok(Self {
            catalogs,
            candidates,
        })
    }

    /// Create a bundle by looking up catalogs for each locale in the fallback
    /// chain.
    ///
    /// Calls `fetch` once per candidate locale, from most specific to least.
    /// The callback returns `Ok(Some(catalog))` when a catalog is available,
    /// `Ok(None)` when none exists for that locale, or `Err(e)` to abort.
    /// Returns [`LookupError::MissingLocaleCatalog`] if no candidate produced
    /// a catalog.
    pub fn from_lookup<E>(
        locale: &Locale,
        mut fetch: impl FnMut(&Locale) -> Result<Option<MessageCatalog>, E>,
    ) -> Result<Self, LookupError<E>> {
        let candidates = locale_candidates(locale);
        let mut catalogs = Vec::new();
        for candidate in &candidates {
            match fetch(candidate) {
                Ok(Some(catalog)) => catalogs.push(catalog.catalog),
                Ok(None) => {}
                Err(e) => return Err(LookupError::Fetch(e)),
            }
        }
        if catalogs.is_empty() {
            return Err(LookupError::MissingLocaleCatalog);
        }
        Ok(Self {
            catalogs,
            candidates,
        })
    }

    /// Create a multi-catalog formatter with message-level fallback.
    ///
    /// Catalogs are searched in fallback order (most specific to least).
    /// The host locale for number/date formatting is derived from the
    /// target locale's CLDR fallback chain.
    pub fn formatter(&self) -> Result<MessageFormatter<'_>, runtime::FormatError> {
        MessageFormatter::new(self.catalogs.iter(), &self.candidates)
    }
}

/// Error returned by [`CatalogBundle::from_lookup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError<E> {
    /// The user-provided callback returned an error.
    Fetch(E),
    /// No catalog matched any candidate in the fallback chain.
    MissingLocaleCatalog,
}
