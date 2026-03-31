---
id: mfc-0qxs
status: closed
deps: []
links: []
created: 2026-03-12T11:42:41Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Add explicit SourceKind::Generated for compiler/resource provenance instead of using ad hoc Other("generated").

## Design

Introduce a dedicated Generated source kind, use it in docs/tests/examples, and keep Other(String) for true custom source kinds.

## Acceptance Criteria

SourceKind exposes Generated, docs/examples use it where appropriate, tests cover formatting/serialization expectations, and validation passes.

## Notes

- Added `SourceKind::Generated` to distinguish in-process/generated inputs from language-specific `Rust` sources and ad hoc `Other("generated")` conventions.
- Updated compiler docs/examples and structured-input tests to use `Generated` where the source is compiler-facing generated data.
- Kept adapter/container source kinds as explicit `Other(...)` values because they still represent concrete external container formats rather than generic generated input.

## Validation

- `cargo fmt --all`
- `typos`
- `taplo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
