---
id: mf-u9pj
status: closed
deps: []
links: []
created: 2026-02-23T05:08:29Z
type: chore
priority: 3
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Fix dead-code charset predicates: is_text_char, is_simple_start_char

is_text_char adds } (not in spec) and relies on over-inclusive is_content_char. is_simple_start_char excludes @ and | but the spec adds them. Both are dead code (never called from parser.rs). Either fix definitions to match ABNF or remove them if they'll never be used.

