---
id: mfc-4mb0
status: closed
deps: []
links: []
created: 2026-03-12T07:56:12Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Improve resource-backed compile diagnostics so they show both container location and inner MF2 error location

## Design

Keep MF2 inner diagnostics precise while making resource/container errors point back to the resource file value span. Render both container provenance and inner message location/snippet when available for compile_resources and CLI output.

## Acceptance Criteria

compile_resources errors include container context plus inner MF2 context when applicable; CLI output shows both layers; regression tests cover TOML/JSON resource errors

## Notes

### Implementation Summary

- Expanded `message_format_compiler::BuildError` so resource-backed failures can carry
  container source info, message id, original MF2 body, and container-span context through
  one boxed `BuildErrorContext`.
- Added `BuildError::render()` and routed CLI build-error output through it so
  resource-backed compile failures render as:
  - container/header location in the resource file
  - nested MF2 snippet/caret diagnostics for the message body itself when available
- Kept plain single-message MF2 diagnostics on the existing direct `render_with_source(...)`
  path instead of forcing all errors into the nested format.
- Added regression coverage for nested resource diagnostics in the compiler tests.

### Key Decisions / Tradeoffs

- Kept the extra rendering context in a boxed field so `BuildError` stays small enough for
  the compiler API surface under `clippy -D warnings`.
- Did not try to reconstruct MF2 source from the semantic model during later compile
  phases; nested message snippets are only rendered when the original message body is
  actually available. This avoids fake or lossy diagnostics.
- No ADR added: this is a diagnostic-rendering/data-flow refinement, not a new ownership,
  invariant, or public semantic decision.

### Validation

- `typos`
- `taplo fmt --check`
- `cargo fmt --all`
- `cargo test --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo doc --no-deps`
