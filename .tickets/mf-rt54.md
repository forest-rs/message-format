---
id: mf-rt54
status: closed
deps: []
links: []
created: 2026-02-22T04:06:21Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Store borrowed source slices on AST leaf nodes


## Notes

**2026-02-22T04:06:29Z**

Planned follow-up: migrate AST leaf fields from span-only to borrowed source slices (&str) while retaining spans for diagnostics; then remove remaining source.get(span) threading in semantic/frontend.

**2026-02-22T04:18:55Z**

Converted AST leaf payloads to borrowed source slices (&str) with retained spans; threaded lifetimes through parser/semantic/frontend, switched key/value/name accesses to leaf fields, and kept all workspace gates green.
