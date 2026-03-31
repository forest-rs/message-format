---
id: mf-nc6y
status: closed
deps: []
links: []
created: 2026-02-22T04:06:21Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Materialize pattern segments in AST and remove frontend cursor reconstruction


## Notes

**2026-02-22T04:06:25Z**

Added PatternSegmentNode to PatternNode, parser now materializes text/expression segments, frontend lowering consumes segments directly, and declaration-head validation moved from semantic to parser.
