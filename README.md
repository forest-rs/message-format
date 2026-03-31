# message-format

Rust workspace for compiling and executing MessageFormat catalogs.

## Workspace

- `message-format-runtime`: `no_std` runtime for loading verified catalogs and formatting messages.
- `message-format-compiler`: compiler from source text to binary catalog bytes.
- `message-format-resource-json`: JSON resource adapters over the compiler resource model.
- `message-format-resource-toml`: TOML resource adapter over the compiler resource model.
- `message-format`: facade crate for `no_std` formatting APIs plus optional compiler APIs.
- `message-format-conformance`: conformance harness for fixtures and golden tests.
- `apps/message-format-cli`: CLI for compiling explicit resource-adapter inputs into one binary catalog.

## CLI

Validate a resource container without writing a catalog:

```sh
cargo run -p message-format-cli -- compile --check \
  --input-format resource-toml \
  messages/app.toml
```

Validate custom functions with a host-agnostic manifest:

```sh
cargo run -p message-format-cli -- compile --check \
  --input-format resource-toml \
  --functions functions.toml \
  messages/app.toml
```

Minimal manifest example:

```toml
[functions.string]
format = true
select = true

[functions."app:link"]
format = true
operand = "string"

[functions."app:link".options.href]
value = "either"
required = true
```

Compile a project-owned TOML resource container:

```sh
cargo run -p message-format-cli -- compile \
  --input-format resource-toml \
  -o app.mfcat \
  messages/app.toml
```

Compile a flat JSON resource catalog:

```sh
cargo run -p message-format-cli -- compile \
  --input-format json-flat \
  -o app.mfcat \
  messages/app.json
```

Compile a Chrome-style `messages.json` resource catalog:

```sh
cargo run -p message-format-cli -- compile \
  --input-format json-chrome \
  -o app.mfcat \
  messages/app.messages.json
```

For a single raw MF2 message string, use the compiler or facade libraries
directly (`message-format-compiler::compile_str` or
`message-format::MessageCatalog::compile_str`). The CLI operates on explicit
resource/container formats rather than guessing message ids from raw MF2 text.

## Specs

- `spec/catalog-binary-format.md`: current binary catalog wire layout and compatibility intent.
- `spec/default-semantics.md`: strict default behavior for missing args, unknown functions, and select defaults.

## Docs

- `docs/architecture-boundaries.md`: crate ownership boundaries and pipeline invariants.
- `docs/error-model.md`: compile/load/format error contracts across crates.
- `docs/runtime-vm-semantics.md`: VM execution invariants and host/error contracts.
- `CONTRIBUTING.md`: contributor workflow, gates, and commit conventions.
