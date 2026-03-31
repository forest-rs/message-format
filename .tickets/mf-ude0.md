---
id: mf-ude0
status: closed
deps: []
links: []
created: 2026-02-26T12:04:16Z
type: feature
priority: 0
assignee: Bruce Mitchener
parent: mf-7ofr
---
# Args and VM value-path cleanup (borrowed-only args, single emit path)


## Notes

**2026-02-26T12:04:49Z**

Slice A from docs/runtime-internals-plan.md: borrowed args/value path hardening and duplicate value-path removal.

**2026-02-26T13:00:08Z**

Completed slice A: runtime Args is borrowed-only (get_ref), owned fallback removed, VM emits via single ref-based value path, and tests updated. Workspace fmt/clippy/tests are green.
