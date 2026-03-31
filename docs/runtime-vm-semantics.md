# Runtime VM Semantics

This document defines the behavioral contract for `message-format-runtime` VM execution.

## Goals

- Document execution invariants that must hold across refactors.
- Clarify where errors are produced and which type reports them.
- Define host integration expectations for function calls.

## Non-goals

- Full opcode-by-opcode binary encoding table (see source constants in `vm.rs`).
- Compiler lowering rules.

## Execution Model

The VM executes validated catalog bytecode with:
- A value stack.
- Output segments that are finalized into a `String`.
- Temporary buffers for function call arguments/options.

Pipeline:
1. `Catalog::from_bytes` decodes and verifies catalog structure.
2. `Formatter::resolve` maps message id to entry PC.
3. `Formatter::format` / `Formatter::format_resolved` executes from entry PC to `OP_HALT`.

## Core Invariants

- Catalog verification rejects malformed control flow before execution.
- Decode never reads beyond bytecode bounds.
- Stack underflow is reported as `FormatError::StackUnderflow`.
- Program-counter overflow/invalid targets are reported as `FormatError::BadPc` or `FormatError::Trap`.
- Unknown host functions are reported as `FormatError::UnknownFunction` unless host overrides behavior.
- Missing arguments are reported as `FormatError::MissingArg` under default semantics.

## Host Contract

`Host::call` receives:
- Function id (`u16`).
- Positional argument values.
- Options as `(key_str_id, value)` pairs.

`Host::call_select` has the same signature as `call` and is invoked by `OP_CALL_SELECT` when a function result feeds into selection dispatch. The default implementation delegates to `call`. Hosts may override it to return `Value::StrRef` for known categories (e.g. plural), avoiding allocation. When the selector is a `StrRef`, `CASE_STR` compares pool IDs directly before falling back to string comparison.

`Host::format_default` may override plain interpolation rendering for values.

Host implementations must be side-effect safe for repeated calls because the VM may execute the same message many times with reused formatter scratch buffers.

## Structured Output (FormatSink)

`Formatter::format_to` and `Formatter::format_to_resolved` dispatch formatting events to a consumer-provided `FormatSink` instead of producing a flat `String`. The sink receives four event types:

- `literal(s)` — literal text from the message pattern (`OP_OUT_SLICE`, `OP_OUT_LIT`).
- `expression(s)` — expression output from interpolation, literal expressions, or function results (`OP_OUT_EXPR`, `OP_OUT_VAL`).
- `markup_open(name, options)` — markup open tag (`OP_MARKUP_OPEN`).
- `markup_close(name, options)` — markup close tag (`OP_MARKUP_CLOSE`).

Self-closing markup (`{#br /}`) emits both `markup_open` and `markup_close` in sequence.

Markup options are provided as string key-value pairs. Option values are formatted via display-style conversion (not through `Host::format_default`), since markup options are structural metadata, not user-visible text.

The string-path methods (`format`, `format_resolved`) produce identical output regardless of markup — markup segments contribute nothing to the finalized string.

## Error Boundaries

- Catalog structural issues: `CatalogError`.
- Execution issues: `FormatError`.
- Decode errors surfaced during formatting: `FormatError::Decode(CatalogError)`.

See `docs/error-model.md` for cross-crate error taxonomy.

## Refactor Guardrails

- Keep verification strict; do not defer structural checks to hot execution paths.
- Preserve scratch-buffer reuse (`stack`, `out_segments`, `call_args`, `call_options`) to avoid allocation regressions.
- Any new opcode must define decode length, verifier behavior, and runtime error mapping in the same change.
