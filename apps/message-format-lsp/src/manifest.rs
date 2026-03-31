// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Workspace manifest discovery and loading.

use std::path::{Path, PathBuf};

use message_format_compiler::FunctionManifest;

/// Well-known manifest file name.
const MANIFEST_FILENAME: &str = "manifest.toml";

/// Try to find and load a function manifest from the workspace root.
///
/// Returns `None` when no manifest file exists or when parsing fails (a
/// warning is logged in the latter case).
pub(crate) fn load_manifest(workspace_root: Option<&Path>) -> Option<FunctionManifest> {
    let root = workspace_root?;
    let path = manifest_path(root);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return None,
    };
    match FunctionManifest::parse(&content) {
        Ok(m) => {
            log::info!("loaded function manifest from {}", path.display());
            Some(m)
        }
        Err(err) => {
            log::warn!("failed to parse {}: {err}", path.display());
            None
        }
    }
}

fn manifest_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(MANIFEST_FILENAME)
}
