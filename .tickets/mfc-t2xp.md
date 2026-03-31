---
id: mfc-t2xp
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:10Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Support \{ and \} escape sequences in text

TR35 §2.1 requires that \{ and \} in text resolve to literal { and }. The compiler currently rejects these as syntax errors.

