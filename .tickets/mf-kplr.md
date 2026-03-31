---
id: mf-kplr
status: closed
deps: []
links: []
created: 2026-02-22T02:06:29Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Decompose compile/mod.rs into focused submodules

Extract bidi, function-spec parsing, and string interning helpers from compile/mod.rs into dedicated compile submodules.


## Notes

**2026-02-22T03:40:02Z**

Completed: split compile entrypoint helpers into focused modules (bidi, function-spec parser, interning, frontend) and moved compiler tests into compile/tests.rs.
