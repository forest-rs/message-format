---
id: mfc-j68y
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:18:26Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Compiler should detect missing selector annotation

TR35 S-23/E-16: each selector in .match MUST directly or indirectly reference a declaration with a function annotation. The compiler currently allows bare .input { $x } as a selector.

