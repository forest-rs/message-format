---
id: mfc-sfff
status: closed
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:17:11Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :number style=percent should use :number function, not separate :percent

TR35 §14 says :number with style=percent should multiply by 100 and format with percent sign. The runtime has a separate :percent function but does not support style=percent on :number itself.

