---
id: mfc-ypd9
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:18:26Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Runtime should fall back to catch-all on selector resolution error

TR35 E-7: when an error occurs while resolving a selector, the selector MUST NOT match any variant key other than catch-all *. The runtime currently returns FormatError::MissingArg instead of falling back.

