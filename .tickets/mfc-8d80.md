---
id: mfc-8d80
status: closed
deps: []
links: []
created: 2026-03-12T10:45:39Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Fix duplicate context in resource-backed diagnostic rendering so CLI output does not print both BuildError context and CompileError message/origin suffixes.

## Design

Keep CompileError display detailed for direct use, but when BuildError already has container/message context, render the inner error in a context-free form. Preserve nested MF2 snippets for resource-backed diagnostics.

## Acceptance Criteria

Resource-backed CLI diagnostics do not duplicate message/source context; tests cover builtin option and expression errors rendered through BuildError; full validation passes.

## Notes

**2026-03-12T10:53:00Z**

Fixed duplicated context in resource-backed diagnostic rendering. `CompileError` now exposes an internal brief renderer without message-id/origin suffixes, and `BuildError` uses that brief form when it already has outer container/message context. Added a regression test for a resource-backed builtin option error so the one-line path does not print both `message "..."` and `at source#...` twice. Validation: `cargo fmt --all`; `typos`; `taplo fmt --check`; `cargo test --workspace --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo doc --no-deps`.
