// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

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

impl runtime::Catalog {
    /// Resolve a message id to a reusable handle.
    pub fn resolve(
        &self,
        message_id: &str,
    ) -> Result<runtime::MessageHandle, runtime::FormatError> {
        runtime::MessageHandle::from_catalog(self, message_id)
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
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let candidates = locale_candidates(locale);
        MessageFormatter::new(core::iter::once(self), &candidates)
    }
}

/// A catalog associated with one locale.
#[derive(Debug, Clone)]
pub struct LocalizedCatalog {
    /// Locale for this catalog.
    pub locale: Locale,
    /// Message catalog payload.
    pub catalog: runtime::Catalog,
}

impl LocalizedCatalog {
    /// Construct a localized catalog pair.
    #[must_use]
    pub fn new(locale: Locale, catalog: runtime::Catalog) -> Self {
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
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let candidates = locale_candidates(locale);
        let mut slots: Vec<Option<runtime::Catalog>> = vec![None; candidates.len()];
        for lc in catalogs {
            if let Some(pos) = candidates.iter().position(|c| *c == lc.locale) {
                slots[pos] = Some(lc.catalog);
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
        mut fetch: impl FnMut(&Locale) -> Result<Option<runtime::Catalog>, E>,
    ) -> Result<Self, LookupError<E>> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let candidates = locale_candidates(locale);
        let mut catalogs = Vec::new();
        for candidate in &candidates {
            match fetch(candidate) {
                Ok(Some(catalog)) => catalogs.push(catalog),
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
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
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
