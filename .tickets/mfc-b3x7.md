---
id: mfc-b3x7
status: closed
deps: []
links: []
created: 2026-03-12T11:21:54Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Retain original MF2 body text on parsed messages so resource-backed validation errors can render nested snippets consistently.

## Design

Store optional original message body text on semantic messages parsed from text/resource inputs and use it when building resource-backed diagnostics after later validation passes.

## Acceptance Criteria

Resource-backed builtin validation errors render the same nested snippet/caret form as parser errors; tests cover the rendered output; validation passes.

## Notes

- Parsed `Message` values now retain optional original MF2 body text for later diagnostic rendering.
- Resource-backed validation errors reuse that stored body text, so parser-time and later validation failures render the same nested container-plus-message snippet form.
- The low-level builder path still leaves `message_source` unset unless a caller opts in.

## Validation

- `cargo fmt --all`
- `typos`
- `taplo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
