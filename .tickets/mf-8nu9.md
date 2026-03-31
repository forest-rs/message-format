---
id: mf-8nu9
status: closed
deps: []
links: []
created: 2026-02-22T03:53:42Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Replace parse-oriented bidi prepass with AST-lowering behavior


## Notes

**2026-02-22T03:53:49Z**

Removed compile/bidi.rs source scanning; default_bidi_isolation is now AST-context-aware lowering for output expressions only. Added tests for rewrite and selector non-rewrite.
