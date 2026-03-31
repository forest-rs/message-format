---
id: mfr-cia3
status: done
deps: []
links: []
created: 2026-02-22T04:22:34Z
type: task
priority: 3
assignee: Bruce Mitchener
---
# Evaluate removing ENTER_SCOPE / LEAVE_SCOPE / LOAD_LOCAL / STORE_LOCAL opcodes

These four opcodes are defined but rejected at load time (catalog.rs:371-376) and trap at runtime (vm.rs:324,356-358). The compiler resolves all .local declarations at compile time and never emits them. Evaluate whether to remove them entirely (freeing opcode space) or keep them reserved for a future locals feature. If removing, also drop the match arms in the execute loop (currently dead code).

