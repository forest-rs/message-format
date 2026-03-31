---
id: mf-fm5e
status: closed
deps: []
links: []
created: 2026-02-23T05:08:34Z
type: feature
priority: 3
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Handle standalone quoted-pattern complex-body

ABNF allows complex-message = o *(declaration o) complex-body where complex-body = quoted-pattern / matcher. A standalone {{Hello world}} with no declarations is a valid complex message. Parser currently treats {{Hello}} as a simple message, parsing {Hello}} as a placeholder expression. This only matters if the caller needs to distinguish complex vs simple messages.

