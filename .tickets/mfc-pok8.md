---
id: mfc-pok8
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:17:17Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :datetime style vs field options mutual exclusivity check

TR35 §15 says :datetime style options (dateStyle/timeStyle) and field options (year, month, day, etc.) are mutually exclusive. Supplying both MUST produce a Bad Option error. The runtime does not implement this check.

