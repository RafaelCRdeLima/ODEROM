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

## Sign conventions

Not written down anywhere user-visible before now -- flagged during the
Reissner-Nordström performance investigation. The formulas
`oderom-components::curvature` actually computes (also in that module's
own doc comment):

```
Gamma^a_bc = 1/2 g^ad (d_b g_dc + d_c g_db - d_d g_bc)
R^a_bcd    = d_c Gamma^a_bd - d_d Gamma^a_bc + Gamma^a_ce Gamma^e_bd - Gamma^a_de Gamma^e_bc
R_bd       = R^a_bad                              (Ricci tensor)
R          = g^bd R_bd                            (Ricci scalar)
```

This is one fixed, non-configurable convention -- not a choice exposed
anywhere. Riemann/Ricci sign conventions genuinely differ across GR
references (independently of metric signature), so a sign mismatch
against some other book or paper does not by itself mean either is
wrong; check that reference's own convention before assuming a bug here.
For a concrete anchor, `oderom christoffel`/`riemann` on
`oderom-cli/tests/fixtures/schwarzschild_ascii.od` (coordinates
`t, r, theta, phi`, indices `0,1,2,3`, signature `(-,+,+,+)`) gives:

```
R[0,1,0,1] = -2*M/r^3
R[0,2,0,2] = M/r - 2*M^2/r^2
R[0,3,0,3] = (M/r - 2*M^2/r^2) * sin(theta)^2
R[1,2,1,2] = -M/(r*(1 - 2*M/r))
R[1,3,1,3] = -M*sin(theta)^2/(r*(1 - 2*M/r))
R[2,3,2,3] = 2*M*r*sin(theta)^2
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

## Marco 5 status

Not originally part of the roadmap's implementation plan -- the user
brought it up "só para contexto" after Marco 4, then explicitly asked to
continue into it ("prossiga"). This marco is a category change from
Marcos 1-4: every prior acceptance criterion was checked by *exact*
structural/symbolic equality (Kretschmann literally equals `48M²/r⁶`
after normalizing, Bianchi's cyclic sum literally extracts to zero).
Marco 5's criterion -- "the holonomy of a geodesic triangle on S² equals
its area, within tolerance" -- requires solving two ODEs (the geodesic
equation and parallel transport) numerically and comparing floats.

Two forks were proposed in DESIGN-M5.md and confirmed with the user
before implementation:

- **D5.1**: "JIT" means an interpreted SSA IR with common-subexpression
  elimination, not literal native machine-code generation. A real JIT
  would need a `cranelift`-class dependency, categorically heavier than
  anything used so far; an SSA IR interpreted in a single forward pass
  delivers the actual goal (compile a symbolic `Expr` once, evaluate it
  thousands of times cheaply during RK4 integration) without one.
  `oderom-jit`'s `compile()` lowers `Expr` to a `Program` (a flat
  `Vec<Op>` in SSA form) via hash-consing during construction --
  structurally-equal subexpressions collapse to the same instruction,
  the same technique `oderom-egraph` uses for its hash-consed e-nodes,
  simpler here since there's no union-find, just a cache.
- **D5.2**: RK4 (4th-order Runge-Kutta), hand-written, no numerical
  dependency -- same "build it, don't pull it in" reasoning as
  Schreier-Sims and the scalar CAS in earlier marcos. The geodesic
  equation `dv^i/dt = -Γ^i_jk v^j v^k` and parallel transport
  `dw^i/dt = -Γ^i_jk v^j w^k` are integrated as *one* coupled system
  (`integrate_geodesic_with_transport` in `oderom-components::holonomy`),
  not geodesic-then-transport as two passes, since RK4's intermediate
  stages need consistent state for both at times between the recorded
  steps.

The acceptance test (`oderom-components/tests/holonomy.rs`) uses the
"octant" triangle on the unit sphere -- vertices at the standard basis
points `(1,0,0)`, `(0,1,0)`, `(0,0,1)`, each side a quarter great circle
-- in one stereographic chart projected from the south pole. By symmetry
the triangle is exactly 1/8 of the sphere's area (`π/2`); by
Gauss-Bonnet, since the unit sphere has Gaussian curvature `K=1`
everywhere, the holonomy angle around any geodesic triangle equals its
area, so `π/2` is also the expected holonomy exactly, not just a
numerically-derived target. The three sides' initial positions and
unit-speed velocities were derived by hand (differentiating the
stereographic projection along each great circle at its starting
vertex) and independently checked to have `|v|_g = 1` before writing any
code -- the test also asserts each side's integrated endpoint lands near
the expected next vertex, which would have caught a mistake in that
derivation. It passed on the first run, at 20,000 RK4 steps per side,
well within the `1e-3` tolerance proposed in DESIGN-M5.md (D5.3).

## UI status (Camada A)

Not a roadmap marco -- the user asked, after Marco 5, whether the project
could show symbolic differential-geometry results at all. It turned out
it couldn't: `oderom_expr::Expr` (the CAS behind Christoffel/Riemann/
Ricci/metric components) had no `Display`, only the derived `Debug`
dump of its enum tree, and the CLI's only subcommand (`canon`) only
reaches Marco 1's abstract tensor layer. Proposed in DESIGN-UI.md as
"Camada A" (readable text, before any decision about a graphical UI),
then given three corrections by the user before implementation:

1. **A `Render` trait with targets, not just `Display`.**
   `oderom_core::render::{Render, Target}` (`Unicode`/`Latex`/`Json`),
   implemented for `Scalar` and `Expr`; `Display for Expr` wraps
   `render(Target::Unicode)`. LaTeX is not an optional target -- it's
   explicitly "a razão de ser do projeto" (the reason the project
   exists). The trait lives in `oderom-core`, the one crate every other
   crate already depends on, so any future type anywhere in the
   workspace can implement it without new inter-crate dependencies.
2. **The real content is elision, not formatting.** Showing a tensor
   like Riemann means showing only its independent components under the
   head's declared symmetry group, annotated with orbit size, with
   identically-zero components collapsed into one count and output
   truncated explicitly -- never all `dim^rank` raw components.
   `oderom_components::render` (`classify_tensor`/`classify_grid` +
   `render_classes`) implements this by reusing the exact
   `Bsgs`/orbit-representative logic `ComponentTensor::set` already uses
   for storage compression -- it lives next to `ComponentTensor`, not in
   the CLI, because "which components are independent" is a property of
   the symmetry group, not of where a result gets printed.
3. **Testing discipline.** No correctness test (Kretschmann, Ricci,
   Marco 3's cross-chart metric agreement, Bianchi, holonomy) compares a
   rendered string -- those all still check `Expr`/structural equality.
   The new renderer tests are golden strings, and are documented inline
   as testing the renderer's output format, not any mathematical claim.

Wiring this into the CLI was left for a follow-up (below) rather than
guessed, since it depended on an open question (DESIGN-UI.md's D-UI.3,
the metric-file format) the user hadn't confirmed when approving Camada
A. A graphical UI is explicitly out of scope for now; the user's working
hypothesis is a Jupyter kernel rather than a standalone GUI, which is the
concrete reason the `Json` target exists already instead of being
deferred.

## UI status (Camada A.2 -- CLI)

D-UI.3 resolved: one language, not two formats -- `chart`/`metric`/
`connection` are new declaration kinds in the same `.od` grammar
`manifold`/`bundle`/`head` already used (`parser::parse_model`, which
replaced `parse_prelude`), and the LaTeX-flavored front-end is not a
parallel parser: it is alternate token spellings inside the *same*
`SCALAR_EXPR` grammar (`/` or `\frac{}{}`, `sin(x)` or `\sin(x)`/
`\sin^2(x)`, `\theta` sharing the exact `GREEK_LETTERS` table the
renderer uses in the other direction) -- both always produce the same
`oderom_expr::Expr`, checked directly
(`expr_parser::tests::ascii_and_latex_lower_to_the_same_ast`) and
end-to-end (`oderom-cli/tests/end_to_end.rs` runs the compiled binary
against two fixture files encoding the same Schwarzschild metric, one
ASCII, one LaTeX, and checks the rendered Kretschmann scalar matches
exactly).

Two design questions the user raised before implementation, both
resolved and documented in DESIGN-UI.md before any code was written:

- **Glued subscript indices (`g_{tt}`, 6.3).** Not resolved by a
  per-chart mode (renaming a coordinate would have silently invalidated
  unrelated, unambiguous lines elsewhere in the same file). Resolved
  per-token instead, by backtracking search over the chart's declared
  coordinate names -- exactly one full decomposition of the expected
  length is accepted; zero is a clear error naming the chart's
  coordinates; two or more lists every reading and points at the comma
  form (`g_{t,r}`), which always works, in any chart, with no search at
  all. Backtracking, not greedy/maximal-munch: a chart with both `r` and
  `rho` still resolves `rhor` to `[rho, r]` even though matching `r`
  first dead-ends one character short.
- **Abstract vs. concrete indices in the same file (6.3b).** What does
  `_{ab}`/`[a,b]` mean in a chart whose coordinates happen to be named
  `a`/`b`? Resolved by grammatical context, never by the index's
  spelling: inside a `metric`/`connection` block every index is a
  concrete coordinate position resolved against that declaration's own
  `chart`; inside a tensor-monomial expression (`canon`'s `R[a,b,c,d]`,
  Marco 1) every index is an abstract contraction label, and no chart is
  ever consulted. The two grammars never share a bracket, so the
  question of what a shared spelling would mean never actually arises.

Also registered (not implemented) while answering "does the components
layer handle an arbitrary metric from a file": the Marco 2 diagonal-only
restriction (D-M2.1, DESIGN-M2.md) excludes null coordinates,
Kerr-like off-diagonal terms, and -- the one the user flagged as the
real future concern -- perturbation theory, since `g + h` is generically
non-diagonal even when the background `g` is diagonal.

Five subcommands, DESIGN-UI.md 6.4: `christoffel`, `riemann`, `ricci`,
`scalar`, `kretschmann`, each taking a `.od` FILE plus `--metric`/
`--connection` (only needed if the file declares more than one; an
explicit `--connection` always wins over an implicit metric),
`--target unicode|latex|json`, and `--max-lines`. `riemann`/`ricci`
register an internal, undeclared symmetry head (Riemann's order-8 group,
or a plain symmetric pair for Ricci) purely to route through
`classify_tensor`'s elision -- which components are independent is a
mathematical fact of rank and symmetry, never something the user
declares. `scalar`/`kretschmann` need `g^ab` and refuse cleanly
(`NeedsMetric`) rather than compute a number from a bare `connection`.

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
