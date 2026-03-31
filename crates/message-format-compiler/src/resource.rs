// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Resource/container model layered above the core MF2 message compiler.
//!
//! MF2 standardizes single-message syntax and runtime semantics, but not an
//! application catalog container format for sets of named messages. Adapter
//! crates parse concrete resource formats and translate them into these shared
//! model types before handing them to the compiler.

use crate::semantic::{DetachedSourceSpan, SourceKind};

/// One named MF2 message body extracted from a resource container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageResource {
    /// Stable message id assigned by the container format.
    pub id: String,
    /// MF2 source for one message body.
    pub source: String,
    /// Optional location within the resource/container source.
    pub origin: Option<ResourceSpan>,
    /// Optional upstream/original provenance supplied by the container.
    pub upstream_origin: Option<DetachedSourceSpan>,
}

impl MessageResource {
    /// Create one named MF2 message resource with no origin metadata.
    #[must_use]
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            origin: None,
            upstream_origin: None,
        }
    }

    /// Attach optional origin metadata for this resource entry.
    #[must_use]
    pub fn origin(mut self, origin: ResourceSpan) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Attach optional upstream/original provenance for this resource entry.
    #[must_use]
    pub fn upstream_origin(mut self, origin: DetachedSourceSpan) -> Self {
        self.upstream_origin = Some(origin);
        self
    }
}

/// Span within one resource/container input before it is registered in the compiler source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSpan {
    /// Start byte offset in the container source.
    pub byte_start: usize,
    /// End byte offset in the container source.
    pub byte_end: usize,
    /// Optional 1-based line number.
    pub line: Option<usize>,
    /// Optional 1-based column number.
    pub column: Option<usize>,
}

/// One logical resource/container source contributing named MF2 message bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInput {
    /// Human-facing display name or path for the container source.
    pub name: String,
    /// Logical source kind for provenance.
    pub kind: SourceKind,
    /// Named MF2 message bodies extracted from the container.
    pub resources: Vec<MessageResource>,
}

impl ResourceInput {
    /// Create an empty resource input with explicit source metadata.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: SourceKind) -> Self {
        Self {
            name: name.into(),
            kind,
            resources: Vec::new(),
        }
    }

    /// Push one named message resource.
    #[must_use]
    pub fn resource(mut self, resource: MessageResource) -> Self {
        self.resources.push(resource);
        self
    }

    /// Push one named MF2 message body.
    #[must_use]
    pub fn message(self, id: impl Into<String>, source: impl Into<String>) -> Self {
        self.resource(MessageResource::new(id, source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_input_builder_collects_named_messages() {
        let input = ResourceInput::new("generated", SourceKind::Generated)
            .message("hello", "Hello")
            .resource(MessageResource::new("bye", "Bye"));

        assert_eq!(input.name, "generated");
        assert_eq!(input.resources.len(), 2);
        assert_eq!(input.resources[0].id, "hello");
        assert_eq!(input.resources[1].id, "bye");
    }
}
