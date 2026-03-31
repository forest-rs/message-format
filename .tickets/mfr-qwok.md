---
id: mfr-qwok
status: done
deps: []
links: []
created: 2026-02-22T04:22:33Z
type: bug
priority: 2
assignee: Bruce Mitchener
---
# Fix StrRef truthiness inconsistency

is_falsey (vm.rs:636) returns false for all Value::StrRef, meaning a StrRef pointing to an empty string in the pool is truthy, while Value::Str("") is falsey. This is a semantic inconsistency. The fix would require is_falsey to have access to the catalog to check the pool string length, or to resolve StrRef before the truthiness check.
