---
id: mfr-ly4n
status: done
deps: [mfr-pm0c, mf-ao77]
links: [mfr-pm0c]
created: 2026-02-22T16:05:15Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Investigate symbol-based host return path for selection functions

## Question

Should the Host trait distinguish between "selection" calls (which return a
category for matching) and "formatting" calls (which return display text)?
Currently both go through the same `call()` → `Value::Str(String)` path, which
means selection functions like `:plural` and `:selectordinal` allocate a fresh
`String` (e.g. `"one"`, `"other"`) just to strcmp it against interned match-arm
keys.

## Motivation

- **Allocation per select**: Every `:plural` / `:selectordinal` invocation
  allocates a small owned `String` that exists solely to be compared against
  match arm keys and then dropped. In a hot formatting loop this is unnecessary
  work.

- **Semantic clarity**: MF2 already distinguishes formatting functions from
  selection functions at the spec level. The Host trait could reflect this
  distinction, making it clear which functions are expected to return a category
  symbol vs. rendered text.

- **Natural fit with FUNCS chunk** (see `mfr-pm0c`): If function metadata moves
  to a dedicated binary section, known return values (plural categories, etc.)
  could be pre-interned at catalog load time, enabling pure symbol/ID comparison
  at match time with zero allocation.

## Possible approaches

### A. Separate `call_select` method on Host

Add a second method like `call_select(fn_id, args, options) -> Result<SymbolId>`
that returns an interned symbol rather than a formatted string. The VM would
call `call_select` when the result feeds into a `MATCH` opcode, and `call` when
the result is emitted to output.

- Pro: Clean separation, zero-alloc selection path, type-safe.
- Con: Two methods to implement for host authors; need to define the symbol
  vocabulary (or let hosts register symbols).

### B. Return an enum with a `Symbol` variant

Extend `Value` (or introduce a `CallResult`) with a `Symbol(&str)` or
`Symbol(u32)` variant that borrows from the catalog's string pool. Selection
functions return `Symbol`, formatting functions return `Str`.

- Pro: Single method, backwards compatible if `Str` still works for selection.
- Con: The host needs access to the string pool to resolve or mint symbol IDs,
  which currently lives in the `Catalog` / VM layer.

### C. VM-side interning of match results

Keep the Host API as-is. After `call()` returns a `Value::Str`, the VM interns
the result against the set of match-arm keys before comparing. This is
essentially a lookup table keyed by the arm strings.

- Pro: No Host API change, simplest implementation.
- Con: Still allocates the `String`; the interning just speeds up comparison.
  Doesn't capture the semantic distinction.

### D. Pre-intern known category sets at catalog load

At load time, scan match arms and build a `BTreeMap<String, u32>` of known
match keys. The host returns `Value::Str` as today, but the VM does a single
hash/btree lookup to convert it to a key ID before matching. Combined with the
FUNCS chunk work, the set of possible return values for each function could be
declared in the binary format.

- Pro: No Host API change, amortizes interning cost, enables future
  optimizations.
- Con: Doesn't eliminate the allocation; adds load-time bookkeeping.

## Decision

**Approach A is the required target design.**

This ticket should implement a distinct selection path in the host interface
(`call_select`) and should not land an implementation that keeps selection
functions on the generic string-returning `call()` path.

Approaches C/D may be useful temporary experiments, but they do not satisfy the
intent of this ticket because they retain allocation on the hot selection path
and blur the formatting-vs-selection contract.

## Benchmark Requirement

Before implementation, add and run a dedicated `wind_tunnel` benchmark for
select-heavy plural/selectordinal execution (high message volume, repeated
category selection). Record baseline and post-change numbers in the benchmark
ticket (`mf-ao77`) and link results from this ticket.
