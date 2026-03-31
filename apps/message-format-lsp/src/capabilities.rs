// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Server capability advertisement.

use lsp_types::{
    OneOf, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions,
};

pub(crate) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(lsp_types::TextDocumentSyncSaveOptions::SaveOptions(
                    lsp_types::SaveOptions {
                        include_text: Some(true),
                    },
                )),
                ..TextDocumentSyncOptions::default()
            },
        )),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    }
}
