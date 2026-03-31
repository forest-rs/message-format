---
id: mf-xdf5
status: closed
deps: []
links: []
created: 2026-02-26T12:04:16Z
type: task
priority: 0
assignee: Bruce Mitchener
parent: mf-7ofr
---
# Benchmark+profiling guardrails for runtime internals changes


## Notes

**2026-02-26T12:04:49Z**

Slice D from docs/runtime-internals-plan.md: benchmark/profiling guardrails and before/after recording for perf-affecting refactors.

**2026-02-26T13:07:21Z**

Bench guardrail work landed: wind_tunnel runtime adapters now avoid unnecessary dyn-dispatch in sink-path benches; reran runtime_sink_paths and recorded current baseline envelope (open_close ~51ns, option_literal ~87ns, option_variable ~111ns).

**2026-02-26T13:07:51Z**

Completed current guardrail slice: benchmark harness paths compile and run after API slimming; sink/format adapter paths now avoid avoidable dyn-dispatch overhead; per-change benchmark deltas recorded in ticket notes for runtime_sink_paths and plain_short_arg checks.
