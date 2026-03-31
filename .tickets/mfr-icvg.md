---
id: mfr-icvg
status: closed
deps: []
links: []
created: 2026-02-22T04:22:18Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Add execution step limit to VM

The VM has no gas/step limit. The load-time verifier guarantees halt reachability but cannot detect infinite loops (e.g. a backward OP_JMP_IF_FALSE that always takes the branch). For untrusted catalogs this is a DoS vector. Add a configurable fuel/step counter to the execute loop that returns FormatError::Trap when exhausted.
