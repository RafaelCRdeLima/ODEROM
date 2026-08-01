//! Property test: **every monomial this project can render must parse
//! back into the same monomial.**
//!
//! Written because the same defect showed up three rounds in a row, each
//! time as a separate one-off bug rather than as an instance of a
//! property nobody was checking:
//!
//! 1. a negative coefficient renders as `+ -1 R[a,b]`, two signs in a
//!    row, which the sum splitter rejected;
//! 2. a monomial with every factor eliminated renders as its bare
//!    coefficient, and a trailing separator (`"4 "`) was emitted;
//! 3. covariant derivatives added a `;` to the grammar, which had to be
//!    taught to both sides independently.
//!
//! Each was found by hand, by noticing output that looked wrong. Two of
//! the three were found only *after* being reported as working. The
//! generator below covers coefficients, multi-factor products, free
//! indices, contractions and derivative indices together, so the class
//! is closed rather than the three known members of it.
//!
//! The invariant is equality of the parsed `Monomial`, not of the
//! rendered string: the renderer is free to choose different dummy-index
//! labels than the input used (it does -- dummies are edges, not names),
//! so string equality would be testing something stricter than the
//! grammar actually promises. What must hold is that no *information* is
//! lost in the cycle.

use oderom_cli::parser::{format_monomial, parse_monomial, parse_model};
use oderom_core::Registry;
use proptest::prelude::*;

const PRELUDE: &str = "
manifold M dim 4
bundle TM on M dim 4
head R : TM*, TM*, TM*, TM* symmetry (1 2)- (3 4)- (1 3)(2 4)+
head g : TM*, TM* symmetry (1 2)+
head eps : TM*, TM*, TM* symmetry antisymmetric
head W : TM*
";

fn registry() -> Registry {
    parse_model(PRELUDE).unwrap().registry
}

/// `(head name, arity)` -- the heads the prelude above declares, at a
/// spread of arities so products mix ranks.
const HEADS: &[(&str, usize)] = &[("R", 4), ("g", 2), ("eps", 3), ("W", 1)];

/// Builds a syntactically valid monomial source string: an optional
/// rational coefficient, one to three factors, each optionally carrying
/// covariant-derivative indices after a `;`, with the index labels
/// assigned so that every one appears exactly once (free) or exactly
/// twice (contracted) -- the two shapes the grammar admits.
fn monomial_source() -> impl Strategy<Value = String> {
    (
        prop::option::of((1i64..9, 1i64..5)),
        prop::collection::vec((0usize..HEADS.len(), 0usize..3), 0..4),
        any::<bool>(),
        any::<u64>(),
    )
        .prop_map(|(coeff, picks, negative, seed)| {
            // A factorless monomial is a bare scalar; it needs a
            // coefficient to be anything at all, so force one. This is
            // the shape the renderer emits for `0` and for the metric
            // trace, and the shape whose absence from an earlier version
            // of this generator let two real round-trip failures survive
            // a passing property test.
            let coeff = if picks.is_empty() { Some(coeff.unwrap_or((1, 1))) } else { coeff };
            // Total slot count, derivative indices included.
            let mut slots: Vec<(usize, usize)> = Vec::new(); // (factor index, slot index)
            let mut shapes: Vec<(usize, usize)> = Vec::new(); // (head index, derivative count)
            for (fi, &(hi, derivs)) in picks.iter().enumerate() {
                let total = HEADS[hi].1 + derivs;
                shapes.push((hi, derivs));
                for s in 0..total {
                    slots.push((fi, s));
                }
            }

            // Deterministically pair up a prefix of the slots (each pair
            // becomes one contracted dummy), leaving the rest free.
            let n = slots.len();
            let pairs = (seed as usize) % (n / 2 + 1);
            let mut labels: Vec<String> = vec![String::new(); n];
            let mut next_free = 0usize;
            for p in 0..pairs {
                let name = format!("d{p}");
                labels[2 * p] = name.clone();
                labels[2 * p + 1] = name;
            }
            for label in labels.iter_mut().skip(2 * pairs) {
                *label = format!("f{next_free}");
                next_free += 1;
            }

            let mut out = String::new();
            if let Some((num, den)) = coeff {
                if negative {
                    out.push('-');
                }
                out.push_str(&num.to_string());
                if den != 1 {
                    out.push('/');
                    out.push_str(&den.to_string());
                }
                out.push(' ');
            } else if negative {
                out.push_str("- ");
            }

            let mut cursor = 0usize;
            for (fi, &(hi, derivs)) in shapes.iter().enumerate() {
                let (name, base_arity) = HEADS[hi];
                if fi > 0 {
                    out.push(' ');
                }
                out.push_str(name);
                out.push('[');
                for s in 0..(base_arity + derivs) {
                    if s == base_arity && derivs > 0 {
                        out.push(';');
                    } else if s > 0 {
                        out.push(',');
                    }
                    out.push_str(&labels[cursor]);
                    cursor += 1;
                }
                out.push(']');
            }
            out
        })
}

/// Diagnostic, not an acceptance test: a property test whose generator
/// mostly produces inputs the parser rejects would pass vacuously, so
/// this reports how many generated sources actually reach the round
/// trip. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn probe_generator_yield() {
    use proptest::test_runner::{Config, TestRunner};
    let mut runner = TestRunner::new(Config { cases: 500, ..Config::default() });
    use std::cell::Cell;
    let parsed_ok = Cell::new(0u32);
    let total = Cell::new(0u32);
    let with_derivative = Cell::new(0u32);
    let with_contraction = Cell::new(0u32);
    let negative = Cell::new(0u32);
    runner
        .run(&monomial_source(), |src| {
            total.set(total.get() + 1);
            let mut reg = registry();
            if parse_monomial(&src, &mut reg).is_ok() {
                parsed_ok.set(parsed_ok.get() + 1);
                if src.contains(';') { with_derivative.set(with_derivative.get() + 1); }
                if src.contains('d') { with_contraction.set(with_contraction.get() + 1); }
                if src.trim_start().starts_with('-') { negative.set(negative.get() + 1); }
            }
            Ok(())
        })
        .unwrap();
    let (total, parsed_ok) = (total.get(), parsed_ok.get());
    println!("gerados={total} parseiam={parsed_ok} ({:.0}%)", 100.0 * parsed_ok as f64 / total as f64);
    println!("  com derivada={}  com contracao={}  coef negativo={}", with_derivative.get(), with_contraction.get(), negative.get());
    assert!(parsed_ok * 2 > total, "generator yield too low: the property would be near-vacuous");
}

proptest! {
    /// Render, re-parse, and require the *same monomial back*. This is
    /// the property the three hand-found bugs were each violating.
    #[test]
    fn rendering_a_monomial_and_parsing_it_back_is_the_identity(src in monomial_source()) {
        let mut reg = registry();
        let parsed = match parse_monomial(&src, &mut reg) {
            Ok(m) => m,
            // The generator can emit an index pattern the grammar
            // rejects for reasons unrelated to rendering; those inputs
            // simply have nothing to say about the round trip.
            Err(_) => return Ok(()),
        };

        let rendered = format_monomial(&parsed, &reg);
        let reparsed = parse_monomial(&rendered, &mut reg)
            .map_err(|e| TestCaseError::fail(format!("rendered `{rendered}` (from `{src}`) does not parse back: {e}")))?;

        prop_assert_eq!(
            &reparsed, &parsed,
            "round trip changed the monomial\n  source:   {}\n  rendered: {}",
            src, rendered
        );
    }

    /// Rendering is idempotent: rendering the re-parsed monomial gives
    /// byte-identical text. Catches a renderer that is stable in meaning
    /// but oscillates in spelling, which would make golden CLI tests
    /// flaky without ever losing information.
    #[test]
    fn rendering_is_idempotent(src in monomial_source()) {
        let mut reg = registry();
        let Ok(parsed) = parse_monomial(&src, &mut reg) else { return Ok(()) };
        let once = format_monomial(&parsed, &reg);
        let Ok(reparsed) = parse_monomial(&once, &mut reg) else {
            return Err(TestCaseError::fail(format!("`{once}` does not re-parse")));
        };
        let twice = format_monomial(&reparsed, &reg);
        prop_assert_eq!(once, twice);
    }
}
