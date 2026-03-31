---
id: mf-vl4a
status: closed
deps: []
links: []
created: 2026-02-26T12:04:16Z
type: feature
priority: 0
assignee: Bruce Mitchener
parent: mf-7ofr
---
# Unify formatter APIs around sink-core with minimal facade helpers


## Notes

**2026-02-26T12:04:49Z**

Slice B from docs/runtime-internals-plan.md: API surface minimization (sink-core runtime, minimal facade helpers).

**2026-02-26T13:00:08Z**

Reduced facade helper surface further: MessageFormatter::format_into is now internal-only; public API remains resolve + format + format_by_id (+ set_fuel).

**2026-02-26T13:00:43Z**

Slice B complete: VM runtime formatter surface is sink-core (resolve + format_to); facade MessageFormatter now exposes minimal convenience (resolve, format, format_by_id, set_fuel) with format_into internal-only; non-core crates use local helper adapters.
