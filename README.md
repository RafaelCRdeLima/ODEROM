# ODEROM

Operational Differential Engine for Riemannian Object Manipulation. See
[DESIGN.md](DESIGN.md) (Marco 1), [DESIGN-M2.md](DESIGN-M2.md) (Marco 2),
[DESIGN-M3.md](DESIGN-M3.md) (Marco 3), and [DESIGN-M4.md](DESIGN-M4.md)
(Marco 4) for the architecture and the representation decisions behind
it. This file tracks what each marco actually delivered.

## Layout

```
oderom-core/        1.1 -- contraction-graph terms, tensor heads, Schreier-Sims BSGS
oderom-types/        1.2 -- the geometric type judgment; Domain/Predicate (Marco 3)
oderom-canon/        1.3 -- Butler-Portugal canonicalization
  tests/               acceptance table + the canon(g*x)==canon(x) property test
  benches/             criterion performance acceptance criteria
oderom-cli/          1.4 -- the `oderom canon` binary
oderom-expr/         2.1 -- symbolic scalar CAS: Expr, diff, normalize; substitute, rationalize (Marco 3)
oderom-components/   2.2 -- Chart, ComponentTensor, Christoffel/Riemann/Ricci; Atlas/transitions (Marco 3)
  tests/               Schwarzschild acceptance tests (Kretschmann, Ricci=0); S^2 stereographic transition
oderom-egraph/       4 -- e-graph, equality saturation, Bianchi identity, cost-based extraction
  tests/               R[a,b,c,d]+R[a,c,d,b]+R[a,d,b,c] extracts to zero
prelude.od           default declarations: M, TM, R (Riemann), g (metric), eps (Levi-Civita_3)
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

## Marco 2 status

**Kretschmann of Schwarzschild = 48M^2/r^6** (`oderom-components/tests/schwarzschild.rs`)
-- passes, along with a second check that Schwarzschild's Ricci tensor and
scalar are identically zero (it's a vacuum solution). Both are computed
from the metric's components by the standard formulas (see
`oderom-components/src/curvature.rs`), with the metric inverted under the
diagonal-only restriction from DESIGN-M2.md's D-M2.1, and the final
covariant Riemann tensor stored via `ComponentTensor`, which keeps only
one `Expr` per symmetry orbit rather than one per raw index tuple (21
independent components in 4D for Riemann's slot symmetry alone, without
imposing the first Bianchi identity -- see the comment at that assertion
in the test for why 21 and not the more commonly quoted 20).

Getting there needed considerably more from `oderom-expr`'s normalizer
than first planned. The original design (`normalize()` folds constants
and collects like terms/bases, nothing else) could not reduce the
Kretschmann sum at all -- Christoffel/Riemann accumulate several distinct
negative powers of `(1 - 2M/r)` that only cancel once brought to a common
denominator, and the resulting numerator only collapses to a single
monomial once recognized as an exact multiple of `(1-2M/r)^n`'s own
expansion. Both are now implemented in `normalize.rs`
(`combine_over_common_denominators`, `divide_by_expanded_power`), and
getting them to coexist with cancellation and sign handling without
recursing forever took three real bugs and fixes along the way -- each
one is documented in place, in `oderom-expr/src/normalize.rs`'s module
docs and the functions themselves, because each was exactly the kind of
thing a future "simplification" could plausibly reintroduce.

## Marco 3 status

**S^2 with two stereographic charts, round metric invariant across the
transition** (`oderom-components/tests/sphere.rs`) -- passes, plus a
sanity check that the checker actually rejects a metric that is *not*
the correct pullback (a flat metric substituted for the round one).
`oderom-types::Domain` gained a `Restricted(Vec<Predicate>)` variant
(symbolic predicates, e.g. `expr != 0`) for a chart's domain of
validity; no solver consumes it -- confirmed with the user that "SMT
obligations" from the original roadmap is out of scope until an
acceptance test actually needs automated proof over inequalities rather
than a pointwise symbolic identity, since a real SMT backend is a much
heavier dependency than anything used so far (see DESIGN-M3.md, D3.1).

This is also where `oderom-expr`'s local-rewriting `normalize()` hit a
real limit: a metric pullback multiplies together *several independent*
sums (the metric's own conformal factor, the transition's Jacobian), and
no ordering of local rules reliably reduces that in general -- a fix
that made one case work (blocking distribution when it can't cancel
anything) broke Kretschmann, which needs exactly the distribution that
fix blocked. Rather than keep patching one local rule against another,
`oderom-expr::rationalize` was added as a separate, principled pass: it
carries an expression's numerator and denominator explicitly through a
single top-down recursion (`a/b + c/d = (ad+bc)/(bd)`, etc.) instead of
trying to re-discover the split from an already-mixed expression by
pattern-matching, and `metric_agrees_across_transition` compares by
cross-multiplying the two sides' rationalized forms rather than
normalizing each and comparing directly. `normalize()` itself was left
exactly as Marco 2 tested it.

## Marco 4 status

The roadmap didn't specify an acceptance criterion for this marco (unlike
Marcos 2 and 3), only the mechanism ("e-grafo e saturação por igualdade;
identidades multi-termo; extração por função de custo"). Proposed and
confirmed with the user: declare `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]`
(the first Bianchi identity's cyclic sum), register the identity with the
e-graph, saturate, and extract -- must be zero with the identity
registered, and must *not* reduce (stays three terms) without it, since
none of the three is related to the others by any of Riemann's own
declared slot symmetries (Bianchi's cyclic permutation has order 3;
Riemann's slot-symmetry group has order 8; by Lagrange's theorem 3 ∤ 8
rules it out, which is exactly why Marco 1's canonicalizer -- pure
slot-permutation symmetry -- can never capture this identity on its own).
Both directions pass (`oderom-egraph/tests/bianchi.rs`).

`oderom-egraph` is a small hand-rolled e-graph (union-find with
congruence closure via `rebuild`, hash-consed `Term`/`Sum` e-nodes,
greedy bottom-up cost extraction) rather than a dependency on the `egg`
crate -- same reasoning as building Schreier-Sims and the scalar CAS from
scratch in earlier marcos: `egg`'s general pattern-rewrite machinery is
a lot of surface for a job that turns out to be "assert a handful of
Riemann-monomial triples sum to zero, then extract." Bianchi is
registered as a specific, hardcoded rule (`apply_bianchi(&mut egraph,
&registry, riemann_head)`), not through a general "declare your own
multi-term identity" mechanism -- confirmed with the user rather than
building the (considerably larger) alternative speculatively.

## Marco 1 status against the acceptance table

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
