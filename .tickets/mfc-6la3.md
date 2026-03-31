---
id: mfc-6la3
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:18:26Z
type: bug
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :string selection should NOT perform NFC normalization

TR35 F-6: :string selection does NOT perform Unicode Normalization. The runtime currently normalizes keys to NFC, causing precomposed and decomposed forms to match when they should not.

