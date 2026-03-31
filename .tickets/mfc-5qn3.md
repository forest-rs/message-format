---
id: mfc-5qn3
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:12Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Duplicate option name should be a compile error

TR35 §7 says duplicate option names (e.g. minimumFractionDigits=1 minimumFractionDigits=2) are a data model error. The compiler currently accepts them silently.

