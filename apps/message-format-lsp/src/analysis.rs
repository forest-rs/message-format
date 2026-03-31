// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Per-document analysis: compile source and collect diagnostics.

use lsp_types::Diagnostic;
use message_format_compiler::{
    CompileOptions, FunctionManifest, ResourceInput, compile_resources,
    compile_resources_with_manifest, compile_with_manifest,
};
use message_format_resource_json::{JsonProfile, parse_json_resource};
use message_format_resource_toml::parse_resource_toml;

use crate::diagnostics::{
    build_error_to_diagnostic, compile_error_to_diagnostic, resource_error_to_diagnostic,
};
use crate::scanner::{self, Declaration, MessageSymbol, VarReference};

/// The kind of document being analyzed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentKind {
    /// Single MF2 message (`.mf2` or unknown).
    SingleMessage,
    /// TOML resource file (`.mf2.toml`).
    ResourceToml,
    /// JSON resource file (`.mf2.json`).
    ResourceJson,
}

/// Cached analysis result for one document.
#[derive(Debug, Clone)]
pub(crate) struct DocumentAnalysis {
    /// LSP diagnostics produced during analysis.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Variable declarations found by the scanner.
    pub(crate) declarations: Vec<Declaration>,
    /// Variable references found by the scanner.
    pub(crate) var_references: Vec<VarReference>,
    /// Message symbols (resource files only).
    pub(crate) message_symbols: Vec<MessageSymbol>,
    /// The document kind that was detected.
    pub(crate) kind: DocumentKind,
}

/// Analyze a document and return diagnostics.
pub(crate) fn analyze(
    source: &str,
    manifest: Option<&FunctionManifest>,
    options: &CompileOptions,
    kind: DocumentKind,
) -> DocumentAnalysis {
    let mut diagnostics = Vec::new();
    let mut message_symbols = Vec::new();

    match kind {
        DocumentKind::SingleMessage => {
            analyze_single(source, manifest, options, &mut diagnostics);
        }
        DocumentKind::ResourceToml => {
            if let Some(input) = parse_toml_input(source, &mut diagnostics) {
                message_symbols = symbols_from_resource(&input);
                compile_resource(input, manifest, options, &mut diagnostics);
            }
        }
        DocumentKind::ResourceJson => {
            if let Some(input) = parse_json_input(source, &mut diagnostics) {
                message_symbols = symbols_from_resource(&input);
                compile_resource(input, manifest, options, &mut diagnostics);
            }
        }
    }

    let var_references = scanner::scan_var_references(source);

    DocumentAnalysis {
        diagnostics,
        declarations: if kind == DocumentKind::SingleMessage {
            scanner::scan_declarations(source)
        } else {
            Vec::new()
        },
        var_references,
        message_symbols,
        kind,
    }
}

/// Determine the document kind from a URI.
pub(crate) fn document_kind_from_uri(uri: &lsp_types::Url) -> DocumentKind {
    let path = uri.path();
    if path.ends_with(".mf2.toml") {
        DocumentKind::ResourceToml
    } else if path.ends_with(".mf2.json") {
        DocumentKind::ResourceJson
    } else {
        DocumentKind::SingleMessage
    }
}

fn analyze_single(
    source: &str,
    manifest: Option<&FunctionManifest>,
    options: &CompileOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let default_manifest = FunctionManifest::default();
    let result = compile_with_manifest(source, *options, manifest.unwrap_or(&default_manifest));
    if let Err(err) = result {
        diagnostics.push(compile_error_to_diagnostic(&err));
    }
}

fn parse_toml_input(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<ResourceInput> {
    match parse_resource_toml("document", source) {
        Ok(input) => Some(input),
        Err(err) => {
            diagnostics.push(resource_error_to_diagnostic(&err.to_string(), err.span()));
            None
        }
    }
}

fn parse_json_input(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<ResourceInput> {
    let profile = detect_json_profile(source);
    match parse_json_resource("document", source, profile) {
        Ok(input) => Some(input),
        Err(err) => {
            diagnostics.push(resource_error_to_diagnostic(&err.to_string(), err.span()));
            None
        }
    }
}

fn compile_resource(
    input: ResourceInput,
    manifest: Option<&FunctionManifest>,
    options: &CompileOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let report = match manifest {
        Some(m) => compile_resources_with_manifest([input], *options, m),
        None => compile_resources([input], *options),
    };
    for err in &report.diagnostics {
        diagnostics.push(build_error_to_diagnostic(err));
    }
}

/// Extract message symbols from a parsed `ResourceInput` using origin spans.
fn symbols_from_resource(input: &ResourceInput) -> Vec<MessageSymbol> {
    input
        .resources
        .iter()
        .map(|res| {
            let (line, span) = match &res.origin {
                Some(origin) => {
                    let line = u32::try_from(origin.line.unwrap_or(1).saturating_sub(1))
                        .unwrap_or(u32::MAX);
                    let span = origin.byte_start..origin.byte_end;
                    (line, span)
                }
                None => (0, 0..0),
            };
            MessageSymbol {
                id: res.id.clone(),
                line,
                span,
            }
        })
        .collect()
}

/// Auto-detect the JSON profile by peeking at the first value in the object.
///
/// If the first value is an object (Chrome-style `{ "message": ... }`), use
/// Chrome; otherwise assume flat string values.
fn detect_json_profile(source: &str) -> JsonProfile {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
        return JsonProfile::Flat; // will fail later with a proper error
    };
    let Some(obj) = value.as_object() else {
        return JsonProfile::Flat;
    };
    if let Some((_, first_value)) = obj.iter().next() {
        if first_value.is_object() {
            return JsonProfile::Chrome;
        }
    }
    JsonProfile::Flat
}
