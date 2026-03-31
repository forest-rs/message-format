---
id: mf-maph
status: closed
deps: []
links: []
created: 2026-02-22T03:47:03Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Move parse-like helper logic from compile into syntax semantics


## Notes

**2026-02-22T03:47:08Z**

Moved compiler-side token parsing helpers into syntax::semantic/literal (variable token parse, function spec option parsing, number literal parse), removed compile/function_spec.rs, and added semantic helper tests.
