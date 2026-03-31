---
id: mfr-pm0c
status: done
deps: [mf-7u4z]
links: [mfr-ly4n]
created: 2026-02-22T04:22:07Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Replace function-in-string-pool encoding with FUNCS chunk

Functions are currently encoded as strings in the string pool (e.g. "number minimumFractionDigits=2") and identified by string pool ID cast to u16. This has several drawbacks: fn_id is u16 but pool IDs are u32 (cap at 65535), BuiltinHost::from_catalog scans the entire string pool parsing every entry for function-ness, message names like "number" get phantom function registrations, and find_string_id does O(n) linear scans for runtime option key resolution. A dedicated FUNCS chunk in the binary format would give functions their own ID space, eliminate the string pool scan at host construction, and remove the u16 truncation risk.
