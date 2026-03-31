# Message Format Catalog Binary Layout (Current)

Status: pre-release working contract. This layout is documented for precision and tests, but is not frozen until the first release.

## Compatibility intent

- Runtime forward-compat: the loader requires `STRS`, `MSGS`, and `CODE`, and ignores unknown chunk tags.
- Compiler forward-compat: the compiler may emit additional chunks in future versions.
- Versioning: major-version mismatch is rejected; minor is currently accepted for the same major.

## Header

All integers are little-endian.

- Bytes `0..8`: magic = `MFCAT\0\0\x01`
- Bytes `8..10`: `version_major` (`u16`)
- Bytes `10..12`: `version_minor` (`u16`)
- Bytes `12..16`: reserved (`u32`, currently zero)
- Bytes `16..20`: `chunk_count` (`u32`)
- Bytes `20..24`: `chunk_table_offset` (`u32`)

## Chunk table

`chunk_count` entries, each 16 bytes:

- Bytes `0..4`: tag (4-byte ASCII)
- Bytes `4..8`: absolute chunk payload offset (`u32`)
- Bytes `8..12`: chunk payload length (`u32`)
- Bytes `12..16`: reserved (`u32`, currently zero)

Current emitted tags:

- `STRS`: string-pool index and bytes
- `LITS`: literal bytes
- `MSGS`: message table
- `CODE`: VM bytecode blob
- `FUNC`: function registry (optional)

## Chunk payload schemas

### `STRS`

- `u32 string_count`
- `string_count` entries of:
  - `u32 offset` (relative to string-bytes area below)
  - `u32 len`
- string bytes area (UTF-8)

### `LITS`

- raw UTF-8 literal bytes (no inner header)

### `MSGS`

- `u32 message_count`
- `message_count` entries of:
  - `u32 name_str_id` (index into `STRS`)
  - `u32 entry_pc` (byte offset into `CODE` body)

### `CODE`

- `u32 code_len`
- `code_len` bytes of VM instructions

### `FUNC` (optional)

- `u32 func_count`
- `func_count` entries of:
  - `u32 name_str_id` (index into `STRS`, e.g. `"number"`)
  - `u32 option_count`
  - `option_count` pairs of:
    - `u32 key_str_id` (index into `STRS`)
    - `u32 value_str_id` (index into `STRS`)

The `FUNC` chunk registers named functions with their static options. The runtime uses it to build the host function table (e.g. `BuiltinHost`). When absent, the function table is empty.

## Loader invariants enforced today

- Header and chunk-table bounds are validated.
- Required chunks (`STRS`, `MSGS`, `CODE`) must exist exactly once. Optional chunks (`LITS`, `FUNC`) may appear at most once.
- Chunk ranges must not overlap and must point to payload bytes (not into the chunk table).
- Instruction decode must be complete and aligned.
- Jump targets must land on instruction boundaries.
- Message entry points must be instruction boundaries and have reachable `HALT`.
- String and literal backing bytes must be valid UTF-8.
- Stack underflow on any reachable path is rejected at catalog load.
