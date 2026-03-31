# Default Semantics Policy (Pre-Release)

Status: current defaults for this pre-release branch.

These defaults are intentionally strict and deterministic. Compatibility modes, if added later, must be explicit opt-in options.

## Runtime defaults

- Missing argument lookup is an error:
  - runtime returns `FormatError::MissingArg(...)`.
- Unknown function id is an error:
  - runtime returns `FormatError::UnknownFunction { fn_id }` (for example via `NoopHost`).

## Compiler defaults

- Select expressions must include a default arm:
  - compile fails with `CompileError::MissingDefaultArm` when no default is present.

## Future flexibility

If looser behavior is needed (for example placeholder output on missing args or passthrough unknown functions), it should be introduced as explicit compile/runtime options with tests and documentation, while preserving strict defaults.
