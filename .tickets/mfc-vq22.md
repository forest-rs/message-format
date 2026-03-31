---
id: mfc-vq22
status: closed
deps: []
links: []
created: 2026-03-12T09:39:22Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Validate builtin enum option literals at compile time and support multiple compile diagnostics

## Design

Add compile-time validation for known builtin option literal domains such as number style. Introduce aggregated build diagnostics for resource/message compilation so callers can receive more than one compiler diagnostic in one pass when inputs are independent.

## Acceptance Criteria

Invalid builtin literal options like style=curren fail at compile time; compile_resources/related APIs can report multiple diagnostics; tests cover both behaviors

## Notes

### Implementation Summary

- Added compiler-side validation for known builtin literal option domains so typos such as
  `:number style=curren` fail at compile time instead of silently falling back to default
  number formatting.
- Added aggregated build-diagnostic APIs:
  - `compile_inputs_report`
  - `compile_resources_report`
  - manifest-bearing variants of both
- Kept the existing single-error convenience APIs by mapping the report variants back to
  their first diagnostic.
- Updated the CLI to use the report APIs so one run can show multiple independent resource
  or input errors instead of stopping at the first message failure.
- Updated the conformance harness classification for the new builtin-option compile error.

### Key Decisions / Tradeoffs

- Validation is intentionally limited to builtin literal options whose runtime value domains
  are already explicit in this implementation (`number`/`integer` `style`/`select`/`notation`,
  `string u:dir`, and date/time style options). This avoids over-claiming option semantics
  that are not yet modeled clearly enough at compile time.
- The multi-diagnostic path is explicit rather than replacing the existing `Result<_, BuildError>`
  entry points. That keeps the calmer convenience surface while still giving tooling and the
  CLI a better bulk-compilation story.
- No ADR added: this is a validation/diagnostic-surface refinement, not a new ownership or
  public semantic boundary decision.

### Validation

- `typos`
- `taplo fmt --check`
- `cargo fmt --all`
- `cargo test --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo doc --no-deps`
