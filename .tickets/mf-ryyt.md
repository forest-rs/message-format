---
id: mf-ryyt
status: closed
deps: []
links: []
created: 2026-02-22T07:01:25Z
type: feature
priority: 0
assignee: Bruce Mitchener
parent: mf-5qns
---
# Add ICU plural/selectordinal builtins with locale-aware categories


## Notes

**2026-02-22T07:09:25Z**

Added runtime ICU4X builtins :plural and :selectordinal using icu_plurals with locale-scoped cardinal/ordinal rules cached in BuiltinHost; added unit tests.
