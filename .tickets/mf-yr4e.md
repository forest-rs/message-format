---
id: mf-yr4e
status: closed
deps: []
links: []
created: 2026-02-23T05:07:51Z
type: bug
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Allow unescaped { and } inside quoted literals

The ABNF quoted-char production includes { and } as valid unescaped characters inside |...|. Currently is_quoted_char excludes them (inherited from is_content_char), causing |foo{bar}| to misparse as an unterminated literal. Fix is_quoted_char in charset.rs to allow { and } inside quoted literals, and update parse_quoted_literal accordingly.

