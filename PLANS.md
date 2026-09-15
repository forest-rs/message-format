# MF2 execution simplification

## Goal

Replace the compiler's parallel, implicit declaration analyses with one explicit
evaluation/use plan, while preserving the landed branch's behavior and improving
or holding runtime performance.

## Non-goals

- Redesign the MF2 syntax or host API.
- Add dependencies or `unsafe` code.
- Preserve internal compiler or bytecode shapes when a smaller shape is clearer.

## Boundary

The compiler owns declaration evaluation order, storage, and format/select use;
the VM executes that plan without rediscovering source intent. Manifest validation,
slot compaction, interning, and lowering consume the same plan.

## Steps

1. Capture LOC, bytecode, and declaration benchmark baselines.
2. Introduce a single declaration-use analysis with explicit invariants.
3. Delete duplicate liveness, alias propagation, and selector-check machinery.
4. Simplify bytecode/runtime paths exposed by the new plan.
5. Compare behavior, LOC, catalog size, allocations, and runtime benchmarks.
6. Run the full workspace and feature-matrix validation.

## Risks

- Eager declarations must retain source-order diagnostics exactly once.
- A declaration can be used for formatting, selection, or both through aliases.
- Numeric exact/category selection must not repeat host resolution or diagnostics.
- `no_std` and no-default-feature builds must remain clean.
