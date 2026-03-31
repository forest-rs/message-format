---
id: mfc-vava
status: done
deps: [mfc-kuf4]
links: []
created: 2026-02-22T19:17:05Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [tr35-compliance]
---
# :number/:integer missing spec options (notation, compactDisplay, useGrouping, numberingSystem, minimumIntegerDigits, significantDigits)

TR35 §14 requires :number and :integer to support options: compactDisplay, notation, numberingSystem, useGrouping, minimumIntegerDigits, minimumSignificantDigits, maximumSignificantDigits, style (percent). The runtime currently only supports minimumFractionDigits and signDisplay (partially).

