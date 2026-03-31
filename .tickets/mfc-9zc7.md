---
id: mfc-9zc7
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:20Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :date/:time/:datetime should produce locale-aware formatted output

TR35 §15 says :date defaults to style=medium, :time to style=short, :datetime to dateStyle=medium timeStyle=short. The runtime currently returns the raw literal string after validation instead of locale-formatted output.

