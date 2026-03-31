---
id: mf-81cx
status: closed
deps: []
links: []
created: 2026-02-22T06:14:12Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Eliminate remaining non-diagnostic source.get usage in frontend/semantic


## Notes

**2026-02-22T06:16:29Z**

Removed remaining non-diagnostic source.get usage in compile/frontend and syntax/semantic by using AST leaf borrowed values; kept diagnostics behavior and all gates green.
