---
id: mfc-d2gh
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:23Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Fallback value rendering for unresolved variables

TR35 §16 says unresolved variables should render as fallback text (e.g. {dollar-sign varname}) in the output rather than returning an error. The runtime currently returns FormatError::MissingArg.

