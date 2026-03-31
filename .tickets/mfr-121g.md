---
id: mfr-121g
status: open
deps: []
links: []
created: 2026-02-22T04:22:34Z
type: task
priority: 3
assignee: Bruce Mitchener
---
# Extract shared utility code into a common crate

Several pure functions are duplicated between the compiler and runtime crates. A shared message-format-core (no_std compatible) crate could eliminate the duplication. Candidates:

- is_valid_number_literal: compiler syntax/literal.rs vs runtime builtin.rs:341
- strip_bidi_controls: compiler syntax/ident.rs vs runtime builtin.rs:298
- format_signed_number / format_signed_string: compiler compile/frontend/local_eval.rs vs runtime builtin.rs:655-674
- truncate_decimal_string / truncate_decimal_text: vm.rs:601 vs builtin.rs:716
- is_integer_literal / is_signed_integer_literal: near-duplicates in vm.rs:589 and builtin.rs:695
- parse_number: builtin.rs:334 (wraps is_valid_number_literal + parse)

All are pure string functions compatible with no_std.

