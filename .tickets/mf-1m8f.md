---
id: mf-1m8f
status: closed
deps: []
links: []
created: 2026-02-23T05:08:13Z
type: bug
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Attribute values should only accept literals, not variables

ABNF: attribute = "@" identifier [o "=" o literal]. The value position only allows literals. Parser's parse_attribute calls parse_option_value() which accepts both OptionValue::Literal and OptionValue::Variable. @attr=$var should be a syntax error but is currently accepted.

