# MF2 differential runner

This runner compares the Rust implementation with the TypeScript implementation
from the neighboring `messageformat` checkout. It executes the WG corpus and a
deterministic set of scalar parameter mutations, then reports rendered-output,
diagnostic, and structured-parts disagreements.

From the workspace root:

```sh
deno run --allow-read --allow-run tools/differential/compare.ts
```

Pass `--all` to include every disagreement in the JSON report instead of only
base cases and one mutated representative per WG file.

The default checkout locations are `../messageformat` and
`../message-format-wg`. Pass those two paths as positional arguments to override
them. Deno executes the TypeScript sources directly; no npm installation or
build output is required.

The process exits unsuccessfully when semantic disagreements are found.
Structured-parts differences are reported separately because implementations
may include additional fields beyond those asserted by the WG fixtures.
