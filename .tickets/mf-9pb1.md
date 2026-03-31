---
id: mf-9pb1
status: closed
deps: [mfc-nh18]
links: [mfc-nh18]
created: 2026-02-23T02:01:35Z
type: task
priority: 1
assignee: Bruce Mitchener
parent: mfc-kuf4
---
# Remove non-spec :plural/:selectordinal builtins after :number select lands

TR35 defines numeric selection on `:number` via `select=plural|ordinal|exact`.
After that support is implemented, remove standalone `:plural` and
`:selectordinal` builtins to keep the public function surface spec-aligned.
