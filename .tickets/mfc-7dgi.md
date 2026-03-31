---
id: mfc-7dgi
status: closed
deps: []
links: []
created: 2026-03-12T10:19:16Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Replace ad hoc bulk compile report entry points with a single structured diagnostics report model and wire CLI/tooling through it without expanding the calm facade surface.

## Design

Keep single-message compile APIs simple. Bulk compiler/tooling paths return one CompileReport carrying diagnostics with severity. CLI and LSP consume that report. Facade bulk helpers stay calm and do not expose extra top-level report APIs.

## Acceptance Criteria

Compiler exposes one structured report type for bulk/resource compilation; explicit *_report entry points are removed; CLI uses the structured report; tests and docs reflect the new model; full workspace validation passes.

## Notes

**2026-03-12T10:42:00Z**

Replaced the ad hoc bulk `*_report` entry points with one structured `CompileReport` model carrying diagnostics plus optional compiled output. Bulk compiler APIs now return `CompileReport`, CLI/tooling consume that report directly, and the calmer facade helpers collapse it back to a simple `Result` by taking the first fatal diagnostic. This keeps room for future warnings/notes without expanding the top-level compiler surface. Updated compiler/resource tests, compiler rustdoc examples, and JSON resource adapter tests to the new model. Validation: `cargo fmt --all`; `typos`; `taplo fmt --check`; `cargo test --workspace --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo doc --no-deps`.
