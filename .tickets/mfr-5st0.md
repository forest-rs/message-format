---
id: mfr-5st0
status: open
deps: []
links: []
created: 2026-02-22T04:22:34Z
type: task
priority: 3
assignee: Bruce Mitchener
tags: [performance]
---
# Investigate using yoke to avoid catalog input copy

Catalog::from_bytes (catalog.rs:148) copies the entire input into an owned Vec<u8>. This means catalog data from mmap or static bytes is always heap-copied. Investigate whether yoke (from icu4x/zerovec) could allow Catalog to borrow from the input bytes while remaining self-referential (indexes point into the owned/borrowed buffer). The tradeoff is lifetime complexity vs. the current simplicity of a 'static self-contained Catalog.

