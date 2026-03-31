# Contributing

Thanks for contributing to `message-format`.

## Workflow

1. Create or pick a ticket in `tk`.
2. Move ticket to `in_progress`.
3. Make a small, reviewable change.
4. Run required gates locally.
5. Commit with a focused message.
6. Add ticket notes and close ticket when complete.

## Ticket Commands

```sh
tk create "Title"
tk start <id>
tk add-note <id> "What changed and why"
tk close <id>
```

## Required Gates

Run these before each commit:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
typos
```

For behavior parity checks:

```sh
cargo run -p message-format-conformance --bin wg_scoreboard
```

## Commit Guidance

- Keep commits small and single-purpose.
- Separate code and ticket state updates into distinct commits when practical.
- Do not bundle unrelated refactors with behavior changes.

## Architecture Rules

- Keep core crates small and explicit.
- Avoid adding production dependencies unless clearly justified.
- Keep benchmark dependencies in `wind_tunnel`, not core crates.
- Update these docs when contracts change:
  - `docs/architecture-boundaries.md`
  - `docs/error-model.md`
  - `docs/runtime-vm-semantics.md`

## Public API and Diagnostics

- Public items should be documented.
- Behavior changes require tests.
- Compiler/runtime error contract changes must update docs in the same change.
