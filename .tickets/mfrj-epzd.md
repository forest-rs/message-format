---
id: mfrj-epzd
status: closed
deps: []
links: []
created: 2026-03-12T11:03:55Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Untitled

Preserve parser spans on resource adapter errors and use them for LSP diagnostics so malformed JSON/TOML containers do not report at 0:0.

## Design

Attach optional ResourceSpan to ResourceJsonError and ResourceTomlError using parser-provided offsets where available. Expose span accessors on the error types. Update the LSP to render parser-level resource diagnostics from those spans instead of treating adapter failures as unlocated strings.

## Acceptance Criteria

Resource adapter parse/type errors preserve spans when available; LSP parser diagnostics use those spans; tests cover JSON and TOML adapter errors plus LSP diagnostic conversion; validation passes.

## Notes

- `ResourceJsonError` and `ResourceTomlError` now preserve optional `ResourceSpan` data for both parser failures and adapter-level shape/type errors.
- The local LSP analysis path now forwards those spans into parser diagnostics instead of pinning malformed resource files to `0:0`.
- This keeps parser/resource diagnostics separate from compiler diagnostics while using the same provenance model.

## Validation

- `cargo fmt --all`
- `typos`
- `taplo fmt --check`
- `cargo test --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo doc --no-deps`
