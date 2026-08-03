# Catalogue of known tensor manipulations

Acceptance corpus for R2–R8 of `DESIGN-TENSOR-ALGEBRA.md`.

Every entry carries its **expected outcome, declared before running it**. That
ordering is the point: reading outcomes without a prior prediction is how three
wrong mechanisms survived a whole diagnosis in this project. An entry whose
prediction turned out to be wrong is recorded as such, with the correction — a
wrong prediction about ODEROM is data about the catalogue's author, not about
ODEROM, and hiding it would defeat the exercise.

Buckets:

- **A** — reproduces.
- **B** — fails, known missing capability; the plan round that covers it is named.
- **C** — fails unexpectedly: should work with what exists today, and does not.
- **D** — wrong answer. A `D` halts the round immediately.

All entries run through `oderom simplify --prelude catalogue/prelude.od`, with the
flags shown. Heads come from [`prelude.od`](prelude.od) in this directory: `R`
(Riemann symmetries), `g` (symmetric), `eps` (totally antisymmetric rank 3), `F`
(antisymmetric rank 2), `S` (symmetric rank 2), `T` (no symmetry), `V`, `xi`, `J`.

---

## Wave one — what exists today

Chosen to exercise the current engine rather than the roadmap: canonicalization,
declared symmetry, dummy-index handling, metric elimination, both Bianchi
identities, metric compatibility, and symmetrization brackets. Entries requiring
Leibniz or substitution are deliberately absent — they can only produce `B`, and
the first wave exists to find `C` and `D`.

| # | flags | input | expected | observed | bucket |
|---|---|---|---|---|---|
| W1-01 | — | `eps[b,a,c]` | `-1 eps[a,b,c]` | `-1 eps[a,b,c]` | A |
| W1-02 | — | `eps[b,c,a]` | `eps[a,b,c]` | `eps[a,b,c]` | A |
| W1-03 | — | `eps[a,a,c]` | `0` | `0` | A |
| W1-04 | — | `F[a,b] S[a,b]` | `0` | `0` | A |
| W1-05 | — | `R[a,b,c,d] + R[b,a,c,d]` | `0` | `0` | A |
| W1-06 | — | `R[a,b,c,d] - R[c,d,a,b]` | `0` | `0` | A |
| W1-07 | — | `R[a,b,c,d] - R[a,c,b,d]` | not zero | `R[a,b,c,d] + -1 R[a,c,b,d]` | A (neg) |
| W1-08 | — | `eps[a,b,c] + eps[a,b,c]` | `2 eps[a,b,c]` | `2 eps[a,b,c]` | A (neg) |
| W1-09 | — | `T[a,b] - T[b,a]` | not zero | `T[a,b] + -1 T[b,a]` | A (neg) |
| W1-10 | — | `S[a,b] - S[b,a]` | `0` | `0` | A |
| W1-11 | — | `R[a,b,a,b] - R[c,d,c,d]` | `0` | `0` | A |
| W1-12 | `--metric g` | `g[a,a]` | `4` | `4` | A |
| W1-13 | `--metric g` | `g[a,b] R[b,c,d,e]` | `R[a,c,d,e]` | `R[a,c,d,e]` | A |
| W1-14 | `--bianchi R` | `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]` | `0` | `0` | A |
| W1-15 | `--bianchi R` | `R[[a,b,c,d]]` | `0` | `1/3 R[a,b,c,d] + -1/3 R[a,c,b,d] + 1/3 R[a,d,b,c]` | **B (R2/R3)** |
| W1-16 | — | `R[[a,b,c,d]]` | not zero | same as W1-15 | A (neg, degenerate — see note) |
| W1-17 | `--bianchi2 R` | `R[a,b,c,d;e] + R[a,b,d,e;c] + R[a,b,e,c;d]` | `0` | `0` | A |
| W1-18 | `--metric-compatible g` | `g[a,b;c]` | `0` | `0` | A |
| W1-19 | — | `g[a,b;c]` | not zero | `g[a,b;c]` | A (neg) |
| W1-20 | `--metric g` | `g[a,b] g[b,c] R[c,d,e,f]` | `R[a,d,e,f]` | `R[a,d,e,f]` | A |

### W1-12 — a wrong prediction, corrected

First predicted `g[a,a] → 4` with no flags. Observed `g[a,a]`. The prediction was
wrong, not the system: trace elimination is a *declared* operation, and
`--metric g` is what declares it. Re-run with the flag, it gives `4`. Recorded
because the discipline is worthless if only the system's misses get written down.

### W1-15 — the same structural gap, on the algebraic identity

`R_[abcd] = 0` is equivalent to the first Bianchi identity given Riemann's
symmetries. The output is *mathematically* zero — by antisymmetry in the last
pair, `-R[a,c,b,d] = R[a,c,d,b]`, so the three terms are one third of the Bianchi
cyclic sum — but it is not reduced.

This is the sum-arity limitation already recorded: identities match structurally
against a `Sum` node of exactly *k* e-classes, so a 24-term antisymmetrization
that collapses to three is not seen as an instance. R2's stated acceptance
criterion is the differential twin of this (`R[a,b,[c,d;e]]`); this is the
algebraic one, and it should be added to R2's acceptance.

### W1-16 — a negative control that cannot currently discriminate

W1-15 and W1-16 produce **identical output**, i.e. `--bianchi` makes no
difference here. The control "passes" only because the positive case also fails.
It carries no information until W1-15 reaches `A`, and must be re-checked then.
Noted rather than quietly counted as a pass.

---

## Wave two

Not yet written. Held deliberately: wave one turned up one `B` worth adding to
R2's acceptance and one degenerate control, and wave two should be written
against that knowledge rather than before it.
