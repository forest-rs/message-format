---
id: mfrj-1zk3
status: closed
deps: []
links: []
created: 2026-03-12T04:59:06Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Refactor JSON adapters into explicit profiles

Reshape message-format-resource-json around explicit JSON profiles instead of ad hoc top-level functions, and document each supported profile with concrete examples and provenance behavior.

## Design

Keep one adapter crate, not one crate per JSON variant. Introduce a profile enum and profile-specific parsing helpers for current flat and structured shapes. Keep current semantics and CLI behavior, but make the crate docs explain that JSON localization formats are a family of incompatible schemas. Document what each supported profile looks like and what provenance is preserved.

## Acceptance Criteria

Public API exposes explicit JSON profiles for current flat and structured formats; crate-level docs describe the supported profiles with examples; tests cover each profile and provenance preservation; workspace validation passes.


## Notes

**2026-03-12T05:02:03Z**

Refactored message-format-resource-json around explicit JsonProfile dispatch, retained the existing flat/structured convenience helpers as wrappers, and rewrote the crate-level docs to explain why JSON is a family of incompatible localization schemas rather than one format. Updated the CLI to call the profile-based entry point. Validation: typos; taplo fmt --check; cargo fmt --all --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --no-deps.
