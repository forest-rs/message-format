---
id: mf-s5fv
status: closed
deps: []
links: []
created: 2026-02-23T05:08:07Z
type: bug
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Skip bidi controls around identifiers in non-variable contexts

ABNF name production is [bidi] name-start *name-char [bidi]. parse_variable correctly skips bidi via skip_bidi_controls(), but parse_identifier (used for function names, option names, attribute names, markup tags, namespace identifiers) calls parse_name directly with no bidi skipping. Function names like {:‎number‏} with surrounding bidi isolates would fail to parse.

