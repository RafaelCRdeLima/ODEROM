<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/oderom-logo-branco.svg">
  <img src="assets/oderom-logo.svg" alt="ODEROM" width="380">
</picture>

Operational Differential Engine for Riemannian Object Manipulation. See
[DESIGN.md](DESIGN.md) (Marco 1), [DESIGN-M2.md](DESIGN-M2.md) (Marco 2),
[DESIGN-M3.md](DESIGN-M3.md) (Marco 3), and [DESIGN-M4.md](DESIGN-M4.md)
(Marco 4) for the architecture and the representation decisions behind
it. This file tracks what each marco actually delivered.

## Começando (uso, não desenvolvimento)

### No navegador, sem instalar nada

**<https://rafaelcrdelima.github.io/ODEROM/>** abre o caderno do ODEROM
dentro do navegador. Não há download, instalador, permissão de
administrador nem dependência de sistema operacional — funciona igual num
laboratório da universidade, num notebook emprestado e numa máquina onde
não se pode instalar nada.

O cálculo roda **na máquina de quem abriu**, compilado para WebAssembly
(crate [`oderom-wasm`](oderom-wasm/)); nada é enviado para servidor
nenhum. O caderno abre com Reissner–Nordström carregado e nada executado:
Shift+Enter roda um bloco.

O botão **Exportar** escreve o resultado como código para colar noutro
programa — `export sympy kretschmann`, `export mathematica riemann g`,
`export sympy geodesic tau`. Vale para qualquer consulta, e o painel
mostra a linha antes de inserir, para que da segunda vez você já saiba
escrevê-la. Na linha de comando é `oderom export sympy kretschmann FILE`.

Duas diferenças em relação ao aplicativo de desktop, ambas impostas pela
plataforma e documentadas em
[`oderom-app/dist/LEIA-ME.md`](oderom-app/dist/LEIA-ME.md): enquanto uma
conta roda, a aba fica parada (a página tem uma thread só), e por isso
"Cancelar" não tem efeito lá. Salvar baixa um arquivo `.oderom`, no mesmo
formato que o aplicativo lê.

### Na linha de comando

Baixe o executável da sua plataforma em
[Releases](https://github.com/RafaelCRdeLima/ODEROM/releases) e rode:

```
oderom load schwarzschild > schwarzschild.od
oderom kretschmann schwarzschild.od          # -> 48*M^2/r^6
oderom scalar schwarzschild.od               # -> 0  (solução de vácuo)
```

No Linux e no macOS, `chmod +x` uma vez antes do primeiro uso. No Windows,
o SmartScreen avisa que o autor é desconhecido — o binário não é assinado.

Métricas já prontas na galeria: `schwarzschild`, `reissnernordstrom`, `frw`,
`desitter`, `antidesitter`. `oderom load` com um nome desconhecido lista
todas. Para uma métrica própria, escreva um arquivo `.od` como o de
[`examples/kerr.od`](examples/kerr.od).

Se uma consulta esbarrar em um limite, a mensagem de erro diz qual opção
aumentar (`--max-nodes`, `--max-denominator-degree`, `--timeout`). Os
limites existem para falhar em segundos em vez de consumir a máquina.

O material didático está em [`docs/`](docs/): um manual do usuário e uma
apostila de cálculo tensorial em Relatividade Geral, com todo exemplo
computado pelo próprio ODEROM.

**O caderno com interface gráfica ainda é compilado da fonte** — ele tem
dependências de sistema por plataforma. Veja o capítulo de instalação do
manual. Os binários publicados são só da linha de comando, que não tem
nenhuma dependência fora do Rust.

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

**Reissner-Nordström** (`f(r) = 1 - 2M/r + Q²/r²`, three terms instead of
Schwarzschild's two) went further than local rewriting could reach at
all: `oderom-expr`'s `normalize()` now routes internally through a
rational-form engine (`Poly`/`RationalFunction`, subresultant PRS,
recursive multivariate polynomial GCD -- see
[DESIGN-RATIONAL-FORM.md](DESIGN-RATIONAL-FORM.md) for the design and
the algorithm) instead of pattern-matching. Kretschmann of
Reissner-Nordström now completes and matches the closed form
`48M²/r⁶ - 96MQ²/r⁷ + 56Q⁴/r⁸` exactly
(`oderom-components/tests/reissner_nordstrom.rs`, a permanent
acceptance fixture, along with the pre-existing check that its Ricci
*scalar* is zero despite the Ricci *tensor* being nonzero -- it's an
electrovac solution, not vacuum). The previous local-rewriting engine
remains in the codebase as `ODEROM_ENGINE=legacy` (an escape hatch and
permanent differential-testing oracle, not dead code), since the new
engine's canonical form is not always structurally identical to the
old one -- both are compared by numeric value, not by exact `Expr`
equality, in `oderom-expr/src/normalize.rs`'s `v1_and_v2_agree`.

**Known limit, not a bug**: some metrics can make the recursive
multivariate GCD's cost blow up. **Free parameter count alone is not
the criterion** -- an earlier version of this note said "3 or more free
parameters" (found via one synthetic four-term, reciprocal probe,
generalized from that single case); real usage falsified it directly: a
2-parameter metric (same count as Reissner-Nordström) with `g_tt`/`g_rr`
*not* reciprocal hung for 60+s on the exact stage RN finishes in under a
second. Four measured data points, not estimates
(`oderom-session/tests/cancellation.rs` holds the two non-obvious ones
as permanent regression fixtures):

| `g_tt`/`g_rr` | free params | Kretschmann |
|---|---|---|
| reciprocal (RN: `f(r)=1-2M/r+Q²/r²`) | 2 (`M`,`Q`) | ~1s |
| reciprocal (`f(r)=1-2M/r+Q²/r²-L²/r³`) | 3 (`M`,`Q`,`L`) | still running past 30s |
| independent, 1 param each (`1-2M/r` vs `1-M/r`) | 1 (`M`) | ~1.2s |
| independent, 2 params total (`1-2M/r+1/r²` vs `1-2M/r+Q²/r²`) | 2 (`M`,`Q`) | still running past 60s |

The real cost driver is how much structural cancellation is available
to `poly_gcd`'s recursive multivariate GCD, not a parameter tally by
itself: the reciprocal ansatz `g_tt·g_rr = -1` (the textbook
`-f dt² + f⁻¹ dr²` form) hands the GCD a large, guaranteed-shared factor
across almost every intermediate term "for free," which is why RN's 2
parameters are cheap but a non-reciprocal metric with the same 2
parameters is not -- reciprocity buys roughly one parameter's worth of
headroom, not immunity. Past that headroom (in either direction), the
recursive GCD is dense; the next algorithmic step would be
modular/sparse GCD (Zippel), which is where external libraries (FLINT,
Symbolica) are the known answer -- see DESIGN-RATIONAL-FORM.md section
6 for the full note. Guardrails keep this safe either way -- never a
hang without recourse, never a wrong answer: the CLI's `--timeout`,
`--max-nodes`, `--max-denominator-degree` for one-shot commands, and
(DESIGN-UI-SESSION.md) the REPL's own `:timeout` plus Ctrl+C, backed by
cancellation checks *inside* `normalize()`/`poly_gcd`/the subresultant
PRS loop (`oderom-expr/src/cancel.rs`), not only between whole
components -- a single component running away is exactly the case that
surfaced this note. Trigger for reopening the external-library
decision: a real problem in this regime blocking actual use, not the
ceiling alone.

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
non-diagonal even when the background `g` is diagonal. The second known
ceiling, orthogonal to this one, is the rational-form normalizer's
recursive-GCD limit described under "Marco 2 status" above (not simply
free-parameter count -- see that section's table and
DESIGN-RATIONAL-FORM.md section 6) -- a metric can trip either
restriction independently of the other.

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

## UI status (notebook, Etapa 3)

Full design in [DESIGN-NOTEBOOK.md](DESIGN-NOTEBOOK.md). A Mathematica-
style block notebook, not the two-panel layout an earlier draft
proposed: the document is a vertical sequence of blocks, each classified
as a declaration or a query by its own leading keyword
(`oderom_cli::parser::classify_block` -- one grammar, never two), never
by a mode the user picks. The `Model` is reconstructed as a unit from
every declaration block's *current* text whenever any one of them
executes -- never an accumulation of individually-run definitions, which
is exactly how invisible state would creep back in. A single broken
declaration block blocks the whole reconstruction, on purpose -- the
same atomicity `:reload` always had, just easier to hit now that
declarations are split into independently-editable blocks.

Two crates:

- **`oderom-notebook`** -- all of the above, fully tested with no
  window (block lifecycle, the three declaration states -- confirmed/
  divergent/error --, error attribution down to which block caused a
  failed reconstruction, save/load). Same relationship to
  `oderom-session` that `oderom-repl` has.
- **`oderom-app`** -- the Tauri shell (Etapa 3a-2): stacked, editable
  blocks with syntax highlighting, Jupyter-style (not Mathematica-style)
  execute keys -- Shift-Enter runs the block and moves focus to the
  next one (or creates one, if it was the last), Ctrl-Enter runs it and
  keeps focus put, Alt-Enter runs it and inserts a new block right
  after regardless of what already follows -- output typeset via KaTeX
  from the existing `Target::Latex`, create/edit/execute/delete a
  block, save/open a notebook, opens with a Reissner-Nordstrom example
  loaded (never blank) and the first block already focused. No Node:
  CodeMirror 5 and KaTeX are vendored files (`oderom-app/dist/vendor/`,
  versions and sources in that directory's own README), not a CDN and
  not an npm dependency -- the window has to open with no network
  access. All geometry/algebra/rendering-format decisions stay in Rust
  (`oderom-notebook`/`oderom-session`/`oderom-cli`); the frontend's one
  judgment call is a display-only split of an already-rendered LaTeX
  string into per-line KaTeX-vs-plain-text (`oderom-app/dist/notebook.js`'s
  own doc comment explains why that one decision lives in JS).

**Visual design** follows a mockup, not defaults (`oderom-app/dist/notebook.css`'s
own header comment has the full reasoning): a numbered gutter (`[n]`/
`[ ]`) is a session-wide Jupyter-style execution counter
(`Notebook::next_execution_count`, `Block::execution_count`) -- `[4]`
means "the fourth execution this session," not "the fourth block," and
an unexecuted block always shows `[ ]`, which is the point: it makes
"nothing recomputes on its own" visible without having to explain it.
The focused block gets a ring (`box-shadow` + border color), never a
filled background, since a filled background fights the code's own
syntax-highlight colors. Output has no card or border -- it sits loose
below the block in a serif font, the way a result sits below a
calculation on paper. The trailing empty block (always present) is
styled with a dashed border and placeholder text as a permanent click
target and reminder of how to start typing. A one-line status bar
spells out the three execute shortcuts. Deliberately excluded: a
background color per block kind (would become noise as the notebook
grows) and a per-block mouse "run" button (would compete with the
keyboard for exactly the gesture a notebook wants to encourage -- the
trailing empty block plus Shift/Ctrl/Alt-Enter already cover every way
to create or run a block). The header is a dark bar showing the
project's symbol (`assets/oderom-simbolo.svg`) and the current
notebook's filename -- the full wordmark isn't used there since the
filename already spells the name out as text right next to it, which
would make it redundant; the full signature
(`assets/oderom-logo.svg`, or `-branco` on dark) is for the README,
the manuals, and any future "about" screen instead. Desktop/taskbar
icons are rasterized from `assets/oderom-icone.svg` by
`assets/gerar-icones.sh` into `oderom-app/src-tauri/icons/`,
referenced by `tauri.conf.json`'s `bundle.icon`.

The identity itself (`assets/marca-oderom.html` is the guide) is a
precessing orbit: ten ellipses, each turned 7 degrees from the last,
drawing an O in perspective with the star at perihelion. That is the
signature of general relativity -- the perihelion advance Newton does
not explain and this program computes -- so the mark states what the
project is for rather than decorating it.

**Tested through the real UI, not just the command handlers**
(`oderom-app/src-tauri/tests/keymap.rs`): calling `execute_block`/
`cm.focus()` directly would prove nothing about the keymap or focus
wiring -- exactly the layer a real, live bug was found in during this
feature's own development (a synthetic `KeyboardEvent` needs its legacy
`keyCode` forced to match what a genuine keypress already has set, or
CodeMirror 5 silently ignores it; confirmed by reading its source, not
assumed). `ODEROM_APP_TEST=1` routes the real compiled binary to
`dist/keytest.html` instead of `index.html` -- identical markup and
`<script>` includes, so it drives the actual `notebook.js`/CodeMirror/
KaTeX, never a reimplementation -- where `keytest.js` fires real,
correctly-shaped DOM events (focus via synthetic mouse clicks, Enter
via keydown events with `keyCode`/`which` forced) and reports back
through a file (`ui_test_report`), the same channel `frontend_ready`
already used. Needs a real display to run (`DISPLAY`/Wayland); skips
itself with a clear message otherwise, so `cargo test --workspace`
still passes in a headless CI runner without one -- not yet wired to
run for real in CI (would need Xvfb or equivalent there), a known gap.

Visible obsolescence, per-stage progress, and Cancel are Etapa 3b, not
yet built -- the three declaration states already exist in
`oderom-notebook` today but aren't styled differently on screen yet.

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
