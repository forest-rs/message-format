---
id: mf-v5rv
status: closed
deps: []
links: []
created: 2026-02-22T01:52:11Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Split compile bytecode lowering into submodule

Move lower_parts/select lowering logic from compile/mod.rs into compile/lowering.rs for clearer module boundaries.


## Notes

**2026-02-22T01:52:42Z**

Moved bytecode lowering pipeline (lower_parts/select/selector + patching) into compile/lowering.rs to reduce compile/mod.rs scope.
