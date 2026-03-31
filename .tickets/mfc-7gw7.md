---
id: mfc-7gw7
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:22Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :time with date-only input should default to 00:00:00

TR35 §15 says when a date-only literal is passed to :time, the missing time component defaults to 00:00:00. The runtime currently returns bad-operand.

