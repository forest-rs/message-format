# Runtime Internals Simplification Plan

## Scope

This plan covers the runtime internals seam around argument/value representation,
VM output/diagnostics flow, and formatter API shape.

Parent ticket: `mf-7for`
Children: `mf-ude0`, `mf-vl4a`, `mf-bmid`, `mf-xdf5`

## Goals

1. Keep the VM core small and sink-first.
2. Minimize copying/allocation in hot formatting paths.
3. Tighten ownership/borrowing boundaries so behavior is explicit and stable.
4. Keep benchmark and profiling visibility while refactoring.

## Non-goals

1. No catalog wire-format changes in this plan (`mf-7u4z` remains separate).
2. No new host semantic features (plural/select behavior is separate spec work).
3. No large public API expansion; only remove/trim where justified.

## Current Baseline (already landed)

1. VM formatter is sink-first at runtime level (`resolve` + `format_to`).
2. Runtime convenience wrappers were removed from VM layer.
3. Diagnostics flow is internalized in VM execution.
4. `Args` moved to borrowed-only lookup (`get_ref`).

## Target Architecture

1. **VM core contract**
   - Input: `MessageHandle`, `Args`, `FormatSink`, optional diagnostics collector.
   - Output: sink events + optional recoverable diagnostics.
   - No string assembly logic in VM core.

2. **Value/args contract**
   - `Args` provides borrowed values only.
   - VM stack owns only when required (stack state / host return values).
   - Single value emission path (reference-based).

3. **Layering**
   - Runtime crate: execution primitives and sink/event semantics.
   - Facade crate (`message-format`): minimal convenience APIs for common usage.
   - Test/benchmark crates: local helper adapters where ergonomic wrappers are needed.

## Work Slices

### Slice A (`mf-ude0`): Args/value path hardening

- Remove remaining duplicate value-path logic.
- Audit clones in `OP_LOAD_ARG`, `OP_OUT_VAL`, select dispatch.
- Keep one clear allocation boundary for fallback/error text generation.
- Add/adjust unit tests for borrowed-args behavior and missing-arg fallback.

Acceptance:
- No owned-args fallback seams in runtime VM.
- All runtime tests and conformance tests pass.

### Slice B (`mf-vl4a`): API surface minimization

- Keep VM formatter surface at sink-core level.
- Keep facade formatter surface to smallest useful subset:
  - resolved handle format
  - by-id format convenience
- Move non-essential convenience wrappers to local helper modules in non-core crates.

Acceptance:
- Runtime public API docs reflect sink-core contract.
- No duplicated helper surfaces across runtime/facade without strong reason.

### Slice C (`mf-bmid`): Allocation reduction pass

- Identify and trim avoidable temporary allocations in:
  - markup option resolution path
  - fallback rendering path
  - string assembly adapter paths
- Prefer borrowed views and caller-provided buffers where possible.

Acceptance:
- No regressions in correctness tests.
- `wind_tunnel` shows no statistically significant regressions in key paths;
  target improvements in sink-heavy and markup paths.

### Slice D (`mf-xdf5`): Perf guardrails + profiling

- Maintain benchmark coverage for:
  - plain short/varying args
  - select/plural modes
  - sink/markup event paths
- Keep profiler entry points (`sample`) for plain and builtin-select hot loops.
- Record before/after metrics in ticket notes for each performance-affecting change.

Acceptance:
- Bench suites compile and run in CI-relevant configs.
- Profiling notes capture top hotspots after each meaningful refactor.

## Risks and Mitigations

1. **Risk**: API churn spills into downstream crates/tests.
   - Mitigation: use local helper adapters in conformance/bench/test crates.

2. **Risk**: small regressions hidden by noisy benches.
   - Mitigation: run focused bench groups with stable warmup/measurement settings and compare deltas.

3. **Risk**: over-optimization hurts readability.
   - Mitigation: prefer simple, commented fast paths; keep data contracts explicit.

## Execution Order

1. Finish Slice A (value/args cleanup) completely.
2. Lock Slice B (API shape) and docs.
3. Run Slice C only with Slice D measurement harness active.
4. End with docs/API pass (`mf-zul9`) for final wording and examples.

## Done Criteria

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features`
4. `cargo doc --workspace --all-features --no-deps`
5. `typos`
6. Tickets updated/closed for completed slices.
