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
  Further split into **B-clean** (fails with a message naming what is missing)
  and **B-dirty** (generic error, partial or confusing output, or *appears* to
  succeed while doing something other than what was asked). A B-dirty is close to
  a `D`: it is the shape that lets a future round build on a result that was
  never real.
- **C** — fails unexpectedly: should work with what exists today, and does not.
- **D** — wrong answer. A `D` halts the round immediately.

**VACUOUS** is an orthogonal marker, not a bucket. A negative control passes
vacuously when it only passes because the positive case it mirrors also fails —
it discriminates nothing, and reading it as green is a mistake. Such an entry
carries `VACUOUS (pending <entry>)` and must be re-checked when that entry turns
`A`.

This is the same family as the de Sitter convention fixture, with one difference
worth keeping straight: de Sitter was *permanent* vacuity — a maximally symmetric
metric can never witness slot-order convention, so no later work fixes it. The
marker here is *temporary* vacuity, which resolves itself when the depended-on
entry reaches `A`.

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
| W1-15 | `--bianchi R` | `R[[a,b,c,d]]` | `0` | `1/3 R[a,b,c,d] + -1/3 R[a,c,b,d] + 1/3 R[a,d,b,c]` | **B-clean (R1b + R2/R3)** |
| W1-16 | — | `R[[a,b,c,d]]` | not zero | same as W1-15 | A (neg) **VACUOUS (pending W1-15)** |
| W1-17 | `--bianchi2 R` | `R[a,b,c,d;e] + R[a,b,d,e;c] + R[a,b,e,c;d]` | `0` | `0` | A |
| W1-18 | `--metric-compatible g` | `g[a,b;c]` | `0` | `0` | A |
| W1-19 | — | `g[a,b;c]` | not zero | `g[a,b;c]` | A (neg) |
| W1-20 | `--metric g` | `g[a,b] g[b,c] R[c,d,e,f]` | `R[a,d,e,f]` | `R[a,d,e,f]` | A |

### W1-12 — a wrong prediction, corrected

First predicted `g[a,a] → 4` with no flags. Observed `g[a,a]`. The prediction was
wrong, not the system: trace elimination is a *declared* operation, and
`--metric g` is what declares it. Re-run with the flag, it gives `4`. Recorded
because the discipline is worthless if only the system's misses get written down.

### W1-15 — a precisely localized gap, not an absence

`R_[abcd] = 0` is equivalent to the first Bianchi identity given Riemann's
symmetries. The output is *mathematically* zero — by antisymmetry in the last
pair, `-R[a,c,b,d] = R[a,c,d,b]`, so the three terms are one third of the Bianchi
cyclic sum — but it is not reduced.

What the system actually did is worth stating precisely, because "not
implemented" is the wrong summary: it expanded the antisymmetrization to 24
terms, collapsed them to three, carried the correct rational coefficients
(`1/3`, `-1/3`, `1/3`) through that collapse, and stopped *exactly* at the
identity match. Everything up to the last step worked.

The last step cannot work yet, and the reason is representational rather than a
matching bug. Those three terms carry rational coefficients, and the coefficient
still lives inside `ENode::Term` -- so `1/3 R[a,b,c,d]` and `R[a,b,c,d]` are
different e-classes and there is no common key on which a k-term identity could
recognise them. **R1b is a prerequisite here, not only R2/R3**: moving the
coefficient into the sum is what makes the match expressible at all.

R2's stated acceptance criterion is the differential twin of this
(`R[a,b,[c,d;e]]`); the algebraic one belongs there too.

### W1-16 — VACUOUS, pending W1-15

W1-15 and W1-16 produce **identical output**, i.e. `--bianchi` makes no
difference here. The control "passes" only because the positive case also fails.
It carries no information until W1-15 reaches `A`, and must be re-checked then.
Noted rather than quietly counted as a pass.

---

## Wave two — the roadmap

Wave one exhausted what exists today, which is why it came back 18 A. Wave two
enters Leibniz, second-derivative commutators, substitution and definitions, so
it is mostly `B` by construction and that is not a poor result. The useful
product here is the **B-clean / B-dirty** split, plus the negative controls that
double as `D` guards: several of these would be *wrong* if they returned zero,
and none did.

| # | flags | input | expected | observed | bucket |
|---|---|---|---|---|---|
| W2-01 | — | `(V[a] V[b]);c` | error naming the gap | `parse error: expected a tensor factor, found Sym('(')` | **B-dirty (R4)** |
| W2-02 | — | `V[a;c] xi[b] + V[a] xi[b;c]` | not zero | `xi[b] V[a;c] + V[a] xi[b;c]` | A |
| W2-03 | — | `V[a;b,c] - V[a;c,b]` | not zero | unchanged, 2 terms | B-clean (R5) · D-guard |
| W2-04 | — | `T[a,b;c,d] - T[a,b;d,c]` | not zero | unchanged, 2 terms | B-clean (R5) · D-guard |
| W2-05 | — | `R[a,b,c,d;e,f] - R[a,b,c,d;f,e]` | not zero | unchanged, 2 terms | B-clean (R5) · D-guard |
| W2-06 | `--metric g` | `F[a,b;c,d] g[a,c] g[b,d]` | not zero | `F[a,b;a,b]` | B-clean (R5) |
| W2-07 | — | `F[[a,b;c]]` | not zero | `1/3 F[a,b;c] + -1/3 F[a,c;b] + 1/3 F[b,c;a]` | A (neg) · see *undeclarable axioms* |
| W2-08 | `--metric g` | `F[a,b;c] g[a,c]` | not zero | `-1 F[b,a;a]` | A |
| W2-09 | — | `xi[(a;b)]` | not zero | `1/2 xi[a;b] + 1/2 xi[b;a]` | A (neg) · see *undeclarable axioms* |
| W2-10 | — | `xi[a;b] + xi[b;a]` | not zero | unchanged | A (neg) |
| W2-11 | — | `G[a,b] - Ric[a,b]` | not zero | unchanged | B-clean (R6) |
| W2-12 | `--metric g` | `G[a,b] g[a,b]` | not zero | `G[a,a]` | B-clean (R6) |
| W2-13 | `--bianchi2 R --metric g` | `R[a,b,c,d;e] g[a,c] g[b,d]` | not zero | `R[a,b,a,b;e]` | B-clean (R2/R3) |
| W2-14 | `--metric g` | `C[a,b,c,d] g[a,c]` | not zero | `-1 C[b,a,a,d]` | A (neg) · see *undeclarable axioms* |
| W2-15 | — | `V[a;b]` | not zero | `V[a;b]` | A (neg) |
| W2-16 | — | `F[a,b;c] + F[b,a;c]` | `0` | `0` | A |
| W2-17 | — | `S[a,b;c] - S[b,a;c]` | `0` | `0` | A |
| W2-18 | — | `T[a,b;c] - T[b,a;c]` | not zero | unchanged | A (neg) |
| W2-19 | — | `eps[a,b,c;d] + eps[b,a,c;d]` | `0` | `0` | A |
| W2-20 | — | `F[a,a;c]` | `0` | `0` | A |
| W2-21 | — | `F[a,b] F[a,b]` | not zero | unchanged | A (neg) |
| W2-22 | — | `F[a,b] S[b,a]` | `0` | `0` | A |
| W2-23 | — | `eps[a,b,c] S[a,b]` | `0` | `0` | A |
| W2-24 | — | `eps[a,b,c] eps[a,b,c]` | not zero | unchanged | A (neg) |
| W2-25 | `--metric g` | `g[a,b] g[a,b]` | `4` | `4` | A |
| W2-26 | `--metric g` | `g[a,b] F[a,b]` | `0` | `0` | A |
| W2-27 | `--metric g` | `R[a,b,c,d] g[a,b]` | `0` | `0` | A |
| W2-28 | `--metric-compatible g` | `g[a,b;c] V[d]` | `0` | `0` | A |

### W2-01 — the one B-dirty

`∇_c(V_a W_b)` has no notation, which is expected: `ENode` has no derivative node
and R4 is where it arrives. What makes this dirty rather than clean is the
message. A user gets

    parse error: expected a tensor factor, found Sym('(')

which reports that the tokenizer disliked a parenthesis. It does not say that the
derivative of a product is not representable, and a reader would reasonably
conclude the syntax is `(...)`-free rather than that the capability is absent.
Every other failure in these two waves either returns a well-formed unreduced
expression or names its cause.

This is not a request to implement R4. It is the observation that when R4 does
arrive, the *error* it replaces is currently indistinguishable from a typo.

### Undeclarable axioms — one root cause behind four entries

W2-07 (`dF = 0`), W2-09 and W2-10 (the Killing equation), and W2-14 (vanishing
Weyl traces) all fail for the same structural reason, and it is not any of R4-R6:

**there is no mechanism to declare an axiom about a head that is not Riemann or
the metric.** `--bianchi`, `--bianchi2` and `--metric-compatible` are each
hard-wired to a specific tensor's specific identity. A user cannot say "this
rank-2 antisymmetric head satisfies `∇_[a F_bc] = 0`", or "this vector is
Killing", or "this rank-4 head is trace-free".

Every one of these four *expanded correctly* -- the antisymmetrization,
symmetrization and metric contraction all did their job and produced the right
unreduced expression. What is missing is only the declaration that would let the
result collapse. Recorded here rather than as four separate roadmap entries,
because it is one gap and the plan does not currently name it.

### Rank-0 heads are not accepted

`head Rs :` fails with `expected an identifier, found Eof`. The Ricci scalar
therefore cannot be declared as a head, which is why no entry here states an
identity involving `R` (the scalar) directly -- `∇_a R = 2∇^b R_ab` and the trace
of the Einstein tensor both need it. Noted as a representational limit, not
filed as a bucket.

---

## Wave three

Not written. The two structural findings above -- undeclarable axioms on
arbitrary heads, and rank-0 heads -- should be resolved or explicitly deferred in
the plan before a wave three is written against them.
