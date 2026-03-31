---
id: mfr-8svm
status: closed
deps: []
links: []
created: 2026-02-22T04:22:33Z
type: task
priority: 3
assignee: Bruce Mitchener
tags: [performance]
---
# Use binary search for message_pc lookup

Catalog::message_pc (catalog.rs:164) does a linear scan of all message entries. The compiler sorts messages by name_str_id via sort_messages, and string IDs are assigned in lexicographic order from BTreeSet iteration, so the message table is in lexicographic name order. Binary search by comparing against catalog.string(entry.name_str_id) should work and would make lookup O(log n) instead of O(n). The Formatter::resolve/MessageHandle pattern amortizes this, but the linear scan is still unnecessary.

