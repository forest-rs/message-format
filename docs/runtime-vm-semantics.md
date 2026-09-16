# Runtime VM Semantics

This document defines the behavioral contract for `message_format::runtime` VM execution.

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

Local slots are private to one message execution. `StoreLocal` initializes the
next dense slot or replaces an initialized slot; `LoadLocal` requires prior
initialization on every reachable path. The verifier checks each entry point
independently and intersects initialization state at control-flow joins.
Formatters reuse the local buffer's capacity, but clear its values before
returning from execution on both success and error.

Declarations that need runtime evaluation are resolved eagerly in source order,
once per message execution. An unused declaration can therefore report an
error: `.local $x = {$missing :number} {{hello}}` renders `hello` and reports
`MissingArg`. Constant literal bindings can still be substituted at compilation.

The compiler emits `CheckSelector` once for each source selector, before variant
dispatch. Each check reports `BadSelector` if the local is a fallback or an
unselectable resolved number. `SelectLocal` compares exact candidates directly.
When keyword candidates require a category from that same stored number,
`ProjectSelect` invokes the host's stored-value projection path. `BuiltinHost`
uses the retained numeric options directly, without reapplying them or
repeating the diagnostic. These instructions require a definitely initialized
slot. Repeated default subtrees share a target; jumps to an ancestor default
close each intervening select scope with `SelectEnd`.

Failed expressions carry `Value::Fallback` through nested calls and declarations.
Failure state is attached to the value rather than to the following instruction.
`Value::FunctionFallback` distinguishes a function that already reported its
failure from a missing or propagated operand, preventing duplicate selector
diagnostics while retaining the current expression's fallback text.
Resolved strings retain raw text and any requested direction metadata
separately; bidi isolation is applied at output, so later annotations consume
the raw text. An optionless `:string` leaves direction unspecified, while the
compiler represents requested default isolation explicitly as `u:dir=auto`.

Optionless built-in `:string` resolution uses `ResolveString`, so its semantic
text and fallback behavior do not depend on a host implementation. A single
selector-only input may use the equivalent direct `SelectStringArg` path; repeated
selectors or values read by a pattern resolve once into a local.

## Host Contract

`Host::call` receives:
- Function id (`u16`).
- Positional argument values.
- A borrowed `FunctionOptions` view. Iterate it, or use `get`, to access
  resolved options. Runtime option presence remains available through
  `was_dynamic` and `was_unresolved`, including when a fallback-valued option
  is omitted from iteration.

Migration note: host implementations written against the former options slice
should accept `FunctionOptions` and replace slice iteration with
`opts.iter()` (or `opts.into_iter()`). Hosts that need to distinguish static
catalog options from runtime options should use `was_dynamic`.

`Host::call_select` has the same signature as `call` and is invoked by `OP_CALL_SELECT` when a function result feeds into selection dispatch. The default implementation delegates to `call`. Hosts may override it to return `Value::StrRef` for known categories (e.g. plural), avoiding allocation. When the selector is a `StrRef`, `CASE_STR` compares pool IDs directly before falling back to string comparison.

`Host::project_select` receives one already resolved declaration value. The
default delegates to `call_select` with no dynamic options. Hosts that retain
selection metadata in their resolved values can override it to compute a
category without decoding or reapplying the declaration's options.

`Host::format_default` may override plain interpolation rendering for values.
`Host::format_default_to` is the direct-output form; `true` means the host
handled the value and may have written output, while `false` means it wrote
nothing and requests the VM's built-in rendering. Its default preserves
`format_default`, while hosts may override it to avoid a temporary owned string.

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

See `docs/error-model.md` for cross-module error taxonomy.

## Refactor Guardrails

- Keep verification strict; do not defer structural checks to hot execution paths.
- Preserve scratch-buffer reuse (`stack`, `out_segments`, `call_args`, `call_options`) to avoid allocation regressions.
- Any new opcode must define decode length, verifier behavior, and runtime error mapping in the same change.
