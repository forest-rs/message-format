---
id: mf-a3iu
status: closed
deps: []
links: []
created: 2026-02-22T01:52:11Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Move literal/text decoding helpers from compile to syntax

Relocate parse_literal_text/decode_text_fragment/quoted-body validation helpers into syntax module to reduce compiler parsing seams.


## Notes

**2026-02-22T01:52:42Z**

Moved literal/text decoding and quoted-body validation helpers from compile/mod.rs to syntax/literal.rs and wired compile lowering to use syntax helpers.
