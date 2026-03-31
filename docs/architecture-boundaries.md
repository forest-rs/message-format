# Architecture Boundaries

This document defines ownership boundaries and invariants across the workspace.

## Goals

- Keep core crates small and long-lived.
- Make ownership explicit so refactors do not leak responsibilities.
- Preserve a strict compile/load/execute pipeline.

## Non-goals

- Full parser grammar specification.
- Binary wire-format byte listing (see `spec/catalog-binary-format.md`).

## Crate Boundaries

### `message-format-compiler`

Owns:
- Source parsing and semantic validation.
- Lowering and catalog emission targeting the shared executable schema.
- Compile-time diagnostics (`CompileError`) with source location context.

Does not own:
- Runtime execution policy.
- Host function behavior at execution time.
- The executable catalog schema itself.

Invariants:
- Emitted catalogs satisfy runtime verifier requirements.
- Compile-time semantic checks happen before bytecode emission.

### `message-format-runtime`

Owns:
- The executable catalog schema shared with the compiler.
- Catalog decoding and verification.
- VM execution and formatting.
- Runtime error contracts (`CatalogError`, `FormatError`).

Does not own:
- Source parsing.
- Compile-time diagnostics rendering.

Invariants:
- Unverified catalogs do not execute.
- The shared schema remains the single source of truth for executable catalog layout.
- VM instructions are validated and executed with bounded, checked state transitions.
- Runtime never assumes host function presence for unknown IDs.

### `message-format`

Owns:
- End-user facade API over compiler/runtime crates.

Does not own:
- Compiler internals or VM internals.

Invariant:
- Re-exports stay thin and avoid introducing alternate semantics.

### `message-format-conformance`

Owns:
- WG fixture loading and suite execution.
- Scoreboard/failure reporting tools for parity tracking.

Does not own:
- Production compile/runtime behavior.

Invariant:
- Harness assertions reflect the shipped public behavior; harness does not patch runtime semantics.

### `wind_tunnel`

Owns:
- Benchmarks and profiling baselines.

Does not own:
- Production runtime/compiler code paths.

Invariant:
- Benchmark dependencies remain isolated from core crates.

## Pipeline Contract

1. **Compile**: source -> semantic validation -> IR -> catalog bytes.
2. **Load**: catalog bytes -> decode/verify -> `Catalog`.
3. **Execute**: `Catalog` + args + host -> formatted output.

A layer may validate earlier-layer outputs, but must not reintroduce earlier-layer responsibilities.

## Change Rules

- If a change moves ownership across crates, update this doc in the same change.
- If a new public error category is introduced, update `docs/error-model.md`.
- If binary compatibility rules change, update `spec/catalog-binary-format.md`.
