---
id: mfc-b8l5
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:18:26Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Compiler should reject declaration without message body

TR35 E-13: a declaration without a message body (e.g. .local $var = { |x| :string } with no {{...}}) is a Syntax Error. The compiler currently treats it as a simple message.

