// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Main server loop and request/notification dispatch.

use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{DocumentSymbolRequest, GotoDefinition, References, Request as _};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeParams, PublishDiagnosticsParams, Url,
};
use message_format_compiler::{CompileOptions, FunctionManifest};

use crate::analysis;
use crate::document::DocumentStore;
use crate::manifest::load_manifest;
use crate::navigate;

/// Language server state.
pub(crate) struct Server {
    connection: Connection,
    documents: DocumentStore,
    manifest: Option<FunctionManifest>,
    compile_options: CompileOptions,
}

impl Server {
    pub(crate) fn new(connection: Connection, params: &InitializeParams) -> Self {
        let workspace_root = workspace_root_path(params);
        let manifest = load_manifest(workspace_root.as_deref());

        Self {
            connection,
            documents: DocumentStore::default(),
            manifest,
            compile_options: CompileOptions::default(),
        }
    }

    /// Run the main message loop until shutdown.
    pub(crate) fn run(&mut self) {
        while let Ok(msg) = self.connection.receiver.recv() {
            match msg {
                Message::Request(req) => {
                    if self.connection.handle_shutdown(&req).unwrap_or(true) {
                        return;
                    }
                    self.handle_request(req);
                }
                Message::Notification(not) => {
                    self.handle_notification(not);
                }
                Message::Response(_) => {}
            }
        }
    }

    fn handle_request(&mut self, req: Request) {
        let id = req.id.clone();
        match req.method.as_str() {
            GotoDefinition::METHOD => {
                self.handle_definition(id, req);
            }
            References::METHOD => {
                self.handle_references(id, req);
            }
            DocumentSymbolRequest::METHOD => {
                self.handle_document_symbol(id, req);
            }
            _ => {
                log::debug!("unhandled request: {}", req.method);
            }
        }
    }

    fn handle_definition(&self, id: RequestId, req: Request) {
        let params: lsp_types::GotoDefinitionParams = match serde_json::from_value(req.params) {
            Ok(p) => p,
            Err(_) => return,
        };
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let result = self.documents.get(&uri).and_then(|doc| {
            doc.analysis
                .as_ref()
                .and_then(|a| navigate::goto_definition(&doc.content, &uri, a, pos))
        });

        let resp = Response::new_ok(id, result);
        self.connection.sender.send(Message::Response(resp)).ok();
    }

    fn handle_references(&self, id: RequestId, req: Request) {
        let params: lsp_types::ReferenceParams = match serde_json::from_value(req.params) {
            Ok(p) => p,
            Err(_) => return,
        };
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let result = self.documents.get(&uri).and_then(|doc| {
            doc.analysis
                .as_ref()
                .and_then(|a| navigate::find_references(&doc.content, &uri, a, pos))
        });

        let resp = Response::new_ok(id, result);
        self.connection.sender.send(Message::Response(resp)).ok();
    }

    fn handle_document_symbol(&self, id: RequestId, req: Request) {
        let params: lsp_types::DocumentSymbolParams = match serde_json::from_value(req.params) {
            Ok(p) => p,
            Err(_) => return,
        };
        let uri = params.text_document.uri;

        let result = self.documents.get(&uri).and_then(|doc| {
            doc.analysis
                .as_ref()
                .map(|a| navigate::document_symbols(&doc.content, a))
        });

        let resp = Response::new_ok(id, result);
        self.connection.sender.send(Message::Response(resp)).ok();
    }

    fn handle_notification(&mut self, not: Notification) {
        match not.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(params) = cast_notification::<DidOpenTextDocument>(not) {
                    self.on_did_open(params);
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(params) = cast_notification::<DidChangeTextDocument>(not) {
                    self.on_did_change(params);
                }
            }
            DidSaveTextDocument::METHOD => {
                if let Ok(params) = cast_notification::<DidSaveTextDocument>(not) {
                    self.on_did_save(params);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(params) = cast_notification::<DidCloseTextDocument>(not) {
                    self.on_did_close(params);
                }
            }
            _ => {
                log::debug!("unhandled notification: {}", not.method);
            }
        }
    }

    fn on_did_open(&mut self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let content = params.text_document.text;
        self.documents.open(uri.clone(), version, content);
        self.reanalyze_and_publish(&uri);
    }

    fn on_did_change(&mut self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        // We use full sync, so there is exactly one change covering the whole document.
        if let Some(change) = params.content_changes.into_iter().next() {
            self.documents.change(&uri, version, change.text);
            self.reanalyze_and_publish(&uri);
        }
    }

    fn on_did_save(&mut self, params: DidSaveTextDocumentParams) {
        // Re-analyze on save in case the client sent included text.
        let uri = params.text_document.uri;
        if let Some(text) = params.text {
            if let Some(doc) = self.documents.get(&uri) {
                let version = doc.version;
                self.documents.change(&uri, version, text);
            }
        }
        self.reanalyze_and_publish(&uri);
    }

    fn on_did_close(&mut self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        // Clear diagnostics before removing the document.
        self.publish_diagnostics(&uri, Vec::new());
        self.documents.close(&uri);
    }

    fn reanalyze_and_publish(&mut self, uri: &Url) {
        let Some(doc) = self.documents.get(uri) else {
            return;
        };

        let kind = analysis::document_kind_from_uri(uri);
        let result = analysis::analyze(
            &doc.content,
            self.manifest.as_ref(),
            &self.compile_options,
            kind,
        );
        let diagnostics = result.diagnostics.clone();

        if let Some(doc) = self.documents.get_mut(uri) {
            doc.analysis = Some(result);
        }

        self.publish_diagnostics(uri, diagnostics);
    }

    fn publish_diagnostics(&self, uri: &Url, diagnostics: Vec<lsp_types::Diagnostic>) {
        let params = PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version: None,
        };
        let not = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
        self.connection.sender.send(Message::Notification(not)).ok();
    }
}

fn cast_notification<N: lsp_types::notification::Notification>(
    not: Notification,
) -> Result<N::Params, serde_json::Error> {
    serde_json::from_value(not.params)
}

fn workspace_root_path(params: &InitializeParams) -> Option<PathBuf> {
    // Prefer workspace_folders (current spec).
    if let Some(ref folders) = params.workspace_folders {
        if let Some(folder) = folders.first() {
            return folder.uri.to_file_path().ok();
        }
    }
    // Fall back to deprecated fields for older clients.
    #[allow(deprecated, reason = "fallback for older clients")]
    if let Some(ref uri) = params.root_uri {
        return uri.to_file_path().ok();
    }
    #[allow(deprecated, reason = "fallback for older clients")]
    params.root_path.as_ref().map(PathBuf::from)
}
