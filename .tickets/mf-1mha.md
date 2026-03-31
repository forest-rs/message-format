---
id: mf-1mha
status: closed
deps: []
links: []
created: 2026-02-23T05:08:25Z
type: bug
priority: 3
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# escaped-char should only allow the four spec-defined sequences

ABNF: escaped-char = backslash ( backslash / "{" / "|" / "}" ). Only \\, \{, \|, \} are valid. Parser's parse_quoted_literal and build_pattern_node accept backslash followed by any character. \a, \n, \t etc. are silently consumed rather than flagged as syntax errors.

