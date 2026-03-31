---
id: mf-0r1c
status: closed
deps: [mfc-sfff]
links: [mfc-sfff]
created: 2026-02-23T02:01:43Z
type: task
priority: 1
assignee: Bruce Mitchener
parent: mfc-kuf4
---
# Remove non-spec :percent builtin after :number style=percent lands

TR35 percent behavior belongs on `:number` through `style=percent`.
After `mfc-sfff` lands, remove standalone `:percent` to avoid a non-spec
surface that diverges from the standard model.
