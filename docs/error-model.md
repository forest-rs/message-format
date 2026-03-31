# Message Format Error Model

## First Read

Goal: define how failures are reported across compile, catalog load, and format execution.

Non-goals:
- Defining wire-format bytes (see `spec/catalog-binary-format.md`).
- Mapping every WG error code in this document.

The project reports errors in three layers:
- `CompileError` (compiler crate): source parsing/lowering/encoding failures.
- `CatalogError` (runtime crate): binary catalog decoding/verification failures.
- `FormatError` (runtime crate): execution-time failures while formatting.

In normal usage:
1. Compile source with `message_format_compiler`.
2. Load bytes into `message_format_runtime::Catalog`.
3. Format messages with `message_format_runtime::Formatter`.

Each step has its own error type so callers can decide whether the failure is a build problem, data corruption/problem, or runtime-input problem.

## Second Read

### Compiler (`CompileError`)

`CompileError` is structured for diagnostics:
- Most syntax/lowering variants include `line`, `column`, and optional `expected`/`found`.
- `render_with_source` renders a snippet with caret positioning.
- Capacity/encoding limits are explicit (`TooManyStrings`, `FunctionIdOverflow`).

### Catalog Load (`CatalogError`)

`CatalogError` is for invalid or unsupported byte catalogs:
- Header/version/chunk failures (`BadMagic`, `UnsupportedVersion`, `MissingChunk`).
- Structural bytecode failures (`UnknownOpcode`, `BadJump`, `UnterminatedEntry`).
- UTF-8 and bounds failures (`InvalidUtf8`, `ChunkOutOfBounds`, `BadPc`).

### Runtime Format (`FormatError`)

`FormatError` separates message/input/host/runtime issues:
- Message/arg lookup (`UnknownMessageId`, `MissingArg`).
- VM/runtime invariants (`StackUnderflow`, `BadPc`, `Trap`).
- Host integration (`UnknownFunction`, `HostError`).
- Catalog decode issues wrapped as `Decode(CatalogError)`.

`From<CatalogError> for FormatError` is implemented so decode/verification failures can surface uniformly at format boundaries.

## Glossary

- Catalog: binary payload containing string tables, message table, and bytecode.
- Decode: parsing/verifying catalog bytes before or during execution.
- Trap: runtime condition that aborts current formatting operation.
- Host: function provider used by VM call instructions.
- Diagnostic context: `line`/`column`/`expected`/`found` metadata for compile errors.
