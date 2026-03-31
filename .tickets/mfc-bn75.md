---
id: mfc-bn75
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:18:26Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# Compiler should detect implicit variable redeclaration

TR35 S-13: a variable implicitly declared by first use in a prior declaration MUST NOT be explicitly declared afterward. The compiler currently allows this.

