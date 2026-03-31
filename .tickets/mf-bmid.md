---
id: mf-bmid
status: closed
deps: []
links: []
created: 2026-02-26T12:04:16Z
type: feature
priority: 0
assignee: Bruce Mitchener
parent: mf-7ofr
---
# Allocation contract pass: eliminate avoidable per-format temporary allocations


## Notes

**2026-02-26T12:04:49Z**

Slice C from docs/runtime-internals-plan.md: allocation reduction pass; execute with benchmark/profiling guardrails from mf-xdf5.

**2026-02-26T13:05:07Z**

Reduced runtime sink-path temporary allocations by changing FormatSink markup option payload to structured FormatOption slices (removed transient refs vector), plus pre-sized missing-arg fallback string assembly. Bench check (wind_tunnel runtime_sink_paths): open_close improved ~2.9%; option_literal and option_variable no statistically significant change.

**2026-02-26T13:07:21Z**

Completed this allocation pass slice: (1) FormatSink markup options now use FormatOption payload (removed transient refs vector per markup emit), (2) missing-arg fallback text now uses pre-sized String assembly instead of format!, (3) runtime format_to made generic and wind_tunnel bench adapters made generic to avoid dyn-dispatch overhead in benchmark harness hot path. Bench (runtime_sink_paths): open_close ~51.2ns, option_literal ~87.3ns, option_variable ~110.6ns with improvements vs immediate prior run.
