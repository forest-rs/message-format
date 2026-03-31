---
id: mf-q4qc
status: closed
deps: []
links: []
created: 2026-02-23T05:08:19Z
type: chore
priority: 3
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Fix is_content_char to exclude SP, dot, @, and pipe per ABNF

ABNF content-char explicitly excludes SP (U+0020), '.' (U+002E), '@' (U+0040), and '|' (U+007C). Current is_content_char in charset.rs only excludes backslash, {, }, NUL, HTAB, LF, CR, and noncharacters. The over-inclusion has no runtime impact currently since the affected predicates (is_text_char, is_simple_start_char) are dead code, but should be fixed for correctness if those predicates are ever activated.

