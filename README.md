# ODEROM -- Marco 1

Operational Differential Engine for Riemannian Object Manipulation. See
[DESIGN.md](DESIGN.md) for the architecture and the representation
decisions behind it. This file tracks what Marco 1 (the abstract core:
term representation, type judgment, Butler-Portugal canonicalization, and
the `oderom canon` CLI) actually delivered.

## Layout

```
oderom-core/    1.1 -- contraction-graph terms, tensor heads, Schreier-Sims BSGS
oderom-types/   1.2 -- the geometric type judgment
oderom-canon/   1.3 -- Butler-Portugal canonicalization
  tests/          acceptance table + the canon(g*x)==canon(x) property test
  benches/        criterion performance acceptance criteria
oderom-cli/     1.4 -- the `oderom canon` binary
prelude.od      default declarations: M, TM, R (Riemann), g (metric), eps (Levi-Civita_3)
```

One deviation from DESIGN.md's proposed layout: the integration tests
live under `oderom-canon/tests/` rather than a workspace-root `tests/`,
because the workspace root is a virtual manifest (no crate of its own to
attach a `tests/` directory to) and `oderom-canon` is the natural place
for tests spanning `oderom-core` + `oderom-types` + `oderom-canon`
together.

## Running things

```
cargo test --workspace           # unit + acceptance + the 10,000-case property test
cargo bench -p oderom-canon       # performance acceptance criteria
cargo run -p oderom-cli -- canon "R[a,b,c,d] R[c,d,a,b]"
```

## Status against the acceptance table

**Canonicalization correctness** -- all pass, including the property test:

| Input | Result |
|---|---|
| `R[a,b,c,d]` vs `R[c,d,a,b]` | same canonical form, sign +1 |
| `R[a,b,c,d]` vs `R[b,a,c,d]` | same canonical form, opposite sign |
| `R[a,b,a,b]` vs `R[c,d,c,d]` | identical (dummies are edges, not names) |
| `eps[a,b,c] T[a,b]`, `T` symmetric | detected as zero |
| `R[a,b,c,d] g[a,c] g[b,d]` vs `R[a,b,a,b]` | **left `#[ignore]`d** -- see below |

The one `#[ignore]`d case would require substituting through an explicit
metric (index lowering), reducing a 3-factor monomial to a 1-factor one.
Pure Butler-Portugal canonicalization only reorders/relabels a monomial's
existing slots; it cannot change how many factors a term has. That is
explicit-metric algebra -- Marco 2 territory per DESIGN.md, not a
permutation symmetry a coset search can find. Confirmed with the user
2026-07-19 rather than special-cased.

**Types** -- both pass: contracting `TM` with `TM` (same variance) is
rejected naming both slots; summing terms with different free indices is
rejected.

**Property test** -- `oderom-canon/tests/prop_canon.rs`, 10,000 cases:
for a random monomial and a random element of its own declared symmetry
group, canonicalizing the transformed monomial reproduces the identical
canonical structure, with the coefficient differing from the original by
exactly the accumulated sign of the applied generators. This test caught
a real bug during development (see `oderom-canon/src/coset.rs`'s history:
the stabilizer-chain enumeration was composing transversal representatives
in the wrong order, silently dropping valid group elements from the
search) -- which is exactly the kind of bug this project's canonicalizer
lives or dies by catching, and exactly why the brief asked for this test
before trusting anything else.

**Performance** (criterion, release profile, this machine):

| Case | Budget | Measured |
|---|---|---|
| Riemann degree 3, 6 dummies | < 5 ms | **0.32 ms** |
| Riemann degree 4, 8 dummies | < 50 ms | **15.5 ms** |

Both benchmarks fully contract a cyclic chain of `k` identical-head
Riemann factors (worst case: every factor shares a head with every
other, so the acting group includes the full `S_k` factor-permutation on
top of each factor's own order-8 slot symmetry -- group order 3072 for
`k=3`, 98304 for `k=4`). The current search is full enumeration over the
BSGS-generated stabilizer chain, not yet pruned (see the `// PERF:` note
in `oderom-canon/src/coset.rs`); it meets budget with room to spare at
these orders and pruning was deliberately deferred rather than guessed at.
