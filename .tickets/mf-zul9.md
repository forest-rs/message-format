---
id: mf-zul9
status: closed
deps: []
links: []
created: 2026-02-22T01:43:44Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Public API/docs polish pass

Review crate-level and public-item rustdoc, add missing examples/doctests, and tighten wording around defaults/semantics.


## Notes

**2026-02-25T05:17:11Z**

Reworked facade API around MessageCatalog + MessageFormatter. Removed top-level formatting entry points that required bytes + per-call locale; locale now binds once via formatter_for_locale. Added rustdoc quick-start and facade tests for compile/from-bytes/file paths and locale binding behavior.

**2026-02-25T05:27:21Z**

Renamed runtime/facade formatter APIs for clearer semantics: handle-based methods are now primary (format/format_with_diagnostics/format_to), lookup variants are explicit via *_by_id. Also renamed rich-output path to format_with_diagnostics* and updated compiler/conformance/bench call sites.

**2026-02-25T05:30:09Z**

Adjusted high-level facade consistency: MessageCatalog now exposes resolve(), handle-based format()/format_to() are primary, and lookup paths are explicit as format_by_id()/format_to_by_id(). Added regression test for resolve+handle formatting.

**2026-02-25T05:38:02Z**

Removed host coupling from MessageCatalog::resolve by adding runtime::MessageHandle::from_catalog. Added LocalePolicy (Exact/Lookup) with fallback-aware formatter_with_locale; Lookup tries progressively truncated tags then en-US. Added facade tests for exact failure and lookup fallback.

**2026-02-25T05:44:01Z**

Removed implicit default-locale formatting from MessageCatalog (dropped formatter()/format*/format_to* convenience methods that hid en-US). LocalePolicy::Lookup now truncates requested locale only and no longer appends en-US. Callers must bind an explicit locale via formatter_for_locale/formatter_with_locale.

**2026-02-25T10:36:07Z**

Architecture boundary pass applied: message-format split into catalog/options/formatter modules; added typed Locale API (icu_locale_core) across facade and runtime BuiltinHost; introduced LocalizedCatalog and CatalogBundle for cross-catalog locale fallback; moved locale fallback candidate generation into runtime (locale_fallback_candidates); removed duplicate resolve API from MessageFormatter and kept resolve primary on MessageCatalog; added rustdoc recommended and advanced entrypoint flows.

**2026-02-26T13:09:33Z**

Polished public docs to match current API boundaries after runtime/facade slimming: clarified MessageFormatter::resolve handle reuse semantics; clarified that format/format_by_id ignore recoverable diagnostics by design; clarified runtime Formatter::format_to return contract (recoverable diagnostics vs fatal errors).
