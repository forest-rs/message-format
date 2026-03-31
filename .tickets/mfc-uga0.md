---
id: mfc-uga0
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T16:48:14Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :number should reject Bool operands as bad-operand

TR35 §9 says :number with a non-numeric operand is a bad-operand error. The runtime currently coerces Bool(true) to 1 and Bool(false) to 0.

