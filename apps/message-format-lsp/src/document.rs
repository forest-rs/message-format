// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Open-document state tracking.

use std::collections::HashMap;

use lsp_types::Url;

use crate::analysis::DocumentAnalysis;

/// Tracks the set of open documents and their current content.
#[derive(Debug, Default)]
pub(crate) struct DocumentStore {
    documents: HashMap<Url, DocumentState>,
}

/// State for a single open document.
#[derive(Debug)]
pub(crate) struct DocumentState {
    /// Current full text.
    pub(crate) content: String,
    /// Client-reported version.
    pub(crate) version: i32,
    /// Cached analysis result.
    pub(crate) analysis: Option<DocumentAnalysis>,
}

impl DocumentStore {
    pub(crate) fn open(&mut self, uri: Url, version: i32, content: String) {
        self.documents.insert(
            uri,
            DocumentState {
                content,
                version,
                analysis: None,
            },
        );
    }

    pub(crate) fn change(&mut self, uri: &Url, version: i32, content: String) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.content = content;
            doc.version = version;
            doc.analysis = None;
        }
    }

    pub(crate) fn close(&mut self, uri: &Url) {
        self.documents.remove(uri);
    }

    pub(crate) fn get(&self, uri: &Url) -> Option<&DocumentState> {
        self.documents.get(uri)
    }

    pub(crate) fn get_mut(&mut self, uri: &Url) -> Option<&mut DocumentState> {
        self.documents.get_mut(uri)
    }
}
