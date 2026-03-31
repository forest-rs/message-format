---
id: mf-z45s
status: closed
deps: []
links: []
created: 2026-03-13T00:00:00Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Add facade-level named MessageArgs and make facade formatters resolve names against their active catalog instead of requiring runtime-resolved arg ids.

## Design

Keep runtime id-keyed args as the low-level path, add a calm facade args type keyed by names, update formatter methods/docs/tests to use it, and fix CatalogBundle/locale mismatch by construction.

## Acceptance Criteria

Facade formatting uses named args resolved against the active catalog, bundle formatting no longer silently disagrees with args built from another catalog, tests cover the mismatch case, and workspace validation passes.

## Notes

- Added a facade-owned `message_format::MessageArgs` keyed by names instead of runtime string ids.
- `MessageFormatter` now resolves facade args against its active catalog before formatting, which fixes `CatalogBundle` locale lookup mismatches by construction.
- The runtime id-keyed `runtime::MessageArgs` path remains available as the lower-level advanced interface.
- Unknown facade arg names are ignored during resolution so the facade keeps the existing `FormatError` surface instead of introducing a second argument-resolution error type.

## Validation

- `cargo fmt --all`
- `typos`
- `taplo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
