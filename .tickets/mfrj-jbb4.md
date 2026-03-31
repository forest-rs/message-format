---
id: mfrj-jbb4
status: closed
deps: []
links: []
created: 2026-03-12T05:24:55Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Replace generic structured JSON profile with Chrome JSON

Rename the generic structured JSON profile to Chrome and make the parser/docs/CLI match the actual Chrome messages.json schema with message/description/placeholders-style entries.

## Design

Break the current generic structured JSON profile before publish rather than carrying a misleading name. JsonProfile::Chrome should parse the real top-level object of message entries with required "message" strings and optional metadata. CLI input format becomes json-chrome. Keep flat JSON as the generic profile.

## Acceptance Criteria

Public JSON profile API exposes Flat and Chrome; crate docs show the real Chrome messages.json shape; CLI uses json-chrome instead of json-structured; tests cover Chrome parsing and provenance; validation passes.


## Notes

**2026-03-12T05:28:50Z**

Replaced the generic Structured JSON profile with a real Chrome messages.json profile. JsonProfile now exposes Flat and Chrome, the crate docs show actual Chrome examples, and the parser requires top-level object entries with a string "message" field while ignoring optional metadata like description/placeholders. CLI input format was renamed from json-structured to json-chrome. Validation: typos; taplo fmt --check; cargo fmt --all --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --no-deps.
