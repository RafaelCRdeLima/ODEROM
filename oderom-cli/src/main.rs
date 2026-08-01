use oderom_cli::commands;
use oderom_core::{Monomial, Registry, Scalar};
use oderom_cli::error::CliError;
use oderom_cli::parser;
use std::time::Instant;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let mut args = std::env::args().skip(1);
    let subcommand = args.next().ok_or(CliError::Usage)?;
    match subcommand.as_str() {
        "canon" => run_canon(args),
        "simplify" => run_simplify(args),
        "christoffel" => commands::christoffel_cmd(commands::parse_args(args)?),
        "riemann" => commands::riemann_cmd(commands::parse_args(args)?),
        "ricci" => commands::ricci_cmd(commands::parse_args(args)?),
        "scalar" => commands::scalar_cmd(commands::parse_args(args)?),
        "kretschmann" => commands::kretschmann_cmd(commands::parse_args(args)?),
        "einstein" => commands::einstein_cmd(commands::parse_args(args)?),
        "riccisquare" => commands::riccisquare_cmd(commands::parse_args(args)?),
        "gaussbonnet" => commands::gaussbonnet_cmd(commands::parse_args(args)?),
        "weyl" => commands::weyl_cmd(commands::parse_args(args)?),
        "weylsquare" => commands::weylsquare_cmd(commands::parse_args(args)?),
        "geodesic" => commands::geodesic_cmd(commands::parse_args(args)?),
        "accel" => commands::accel_cmd(commands::parse_args(args)?),
        "export" => run_export(args),
        "load" => run_gallery_load(args),
        _ => Err(CliError::Usage),
    }
}

/// `oderom export TARGET COMMAND FILE [flags]` (Rodada Exportação) --
/// two mandatory positionals ahead of the ordinary `FILE [flags]` every
/// other subcommand already takes, so this can't reuse `parse_args`
/// directly the way every other arm above does: it peels `TARGET` and
/// `COMMAND` off the front itself, then hands the rest to `parse_args`
/// unchanged, exactly as if `oderom export mathematica kretschmann
/// schw.od --metric g` had been `oderom kretschmann schw.od --metric
/// g` with the target/command already known -- so every existing flag
/// (`--metric`/`--connection`/`--param`/`--max-nodes`/...) and
/// redirection (`> out.py`) composes unchanged.
fn run_export(mut args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let export_target = args.next().ok_or(CliError::Usage)?;
    let command_word = args.next().ok_or(CliError::Usage)?;
    commands::export_cmd(export_target, command_word, commands::parse_args(args)?)
}

/// `oderom load NAME` -- the CLI's own access to the spacetime gallery
/// (Rodada Galeria), alongside the notebook's `load` (which pastes the
/// same catalog's entries as editable blocks). Unlike every other
/// subcommand, this one takes no `FILE`: it prints one gallery entry's
/// declarations, as a single valid `.od` file's worth of text
/// (`oderom_cli::gallery::render`), to stdout -- `oderom load
/// desitter > desitter.od` then `oderom scalar desitter.od` composes
/// with every other subcommand exactly the way a hand-written `.od`
/// file would, since the output *is* one.
fn run_gallery_load(mut args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let name = args.next().ok_or(CliError::Usage)?;
    if args.next().is_some() {
        return Err(CliError::Usage);
    }
    let entry = oderom_cli::gallery::find(&name).ok_or(oderom_cli::gallery::UnknownGalleryEntry { name })?;
    println!("{}", oderom_cli::gallery::render(entry));
    Ok(())
}

fn run_canon(mut args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let mut prelude_path = "prelude.od".to_string();
    let mut expr: Option<String> = None;
    while let Some(a) = args.next() {
        if a == "--prelude" {
            prelude_path = args.next().ok_or(CliError::Usage)?;
        } else {
            expr = Some(a);
        }
    }
    let expr = expr.ok_or(CliError::Usage)?;

    let prelude_src = std::fs::read_to_string(&prelude_path)
        .map_err(|source| CliError::Io { path: prelude_path.clone(), source })?;
    let mut model = parser::parse_model(&prelude_src)?;

    // `canon` computes the canonical form of the contraction graph itself
    // (Marco 1.3); it does not run the separate geometric type judgment
    // (Marco 1.2, exercised directly by `oderom-types`'s own test suite).
    // Requiring dual variance on every contraction here would reject the
    // very examples Marco 1's acceptance table exercises, since `R` and
    // `g` are declared fully covariant by default (see `prelude.od`) and
    // Marco 1 has no index raising/lowering to reconcile that with.
    let monomial = parser::parse_monomial(&expr, &mut model.registry)?;

    let start = Instant::now();
    let result = oderom_canon::canonicalize(&monomial, &model.registry)?;
    let elapsed = start.elapsed();

    match result {
        oderom_canon::CanonResult::Zero => println!("0"),
        oderom_canon::CanonResult::Value(c) => {
            let text = parser::format_monomial(&c.monomial, &model.registry);
            let swaps = transposition_count(&c.perm);
            println!(
                "{text}        (sign {}{}, {swaps} slot swap{}, {:.3} ms)",
                if c.sign >= 0 { "+" } else { "" },
                c.sign,
                if swaps == 1 { "" } else { "s" },
                elapsed.as_secs_f64() * 1000.0,
            );
        }
    }
    Ok(())
}

/// Collects like terms in an extracted sum: two monomials that differ
/// only by their rational coefficient are one term, and a group whose
/// coefficients cancel disappears entirely.
///
/// The e-graph canonicalizes each monomial and can prove multi-term
/// identities, but its extraction returns whichever `Polynomial` has
/// fewest *terms* -- it never adds coefficients, so `R[a,b,c,d] +
/// R[b,a,c,d]` came back as `R[a,b,c,d] + -1 R[a,b,c,d]` rather than
/// `0`. That is the single most basic simplification a reader would
/// try first (it is just Riemann's declared antisymmetry), so doing it
/// here is what makes `simplify` mean what its name says. Grouping is
/// by the monomial with its coefficient normalized away -- `Monomial`
/// already derives `Eq`/`Hash` over its full contraction structure, so
/// this is exact structural identity, never a string comparison.
fn collect_like_terms(terms: &[Monomial], registry: &Registry) -> Result<Vec<Monomial>, CliError> {
    let mut order: Vec<Monomial> = Vec::new();
    let mut totals: Vec<Scalar> = Vec::new();
    for term in terms {
        let key = Monomial::try_new(Scalar::ONE, term.factors().into(), term.contractions().clone(), term.free().to_vec(), registry)?;
        match order.iter().position(|k| k == &key) {
            Some(i) => totals[i] = totals[i] + term.coeff(),
            None => {
                order.push(key);
                totals.push(term.coeff());
            }
        }
    }
    let mut out = Vec::new();
    for (key, total) in order.into_iter().zip(totals) {
        if total == Scalar::ZERO {
            continue;
        }
        out.push(Monomial::try_new(total, key.factors().into(), key.contractions().clone(), key.free().to_vec(), registry)?);
    }
    Ok(out)
}

/// `oderom simplify [--prelude PATH] [--bianchi HEAD]... "<sum of monomials>"`
/// -- the abstract-index counterpart of the component-level subcommands:
/// it manipulates a tensor *equation's* left-hand side symbolically,
/// with no chart and no metric anywhere in sight.
///
/// Where `canon` canonicalizes a single monomial (Marco 1, pure
/// slot-permutation symmetry), this reduces a *sum* of them through the
/// e-graph (Marco 4) -- which is where multi-term identities live,
/// because they provably cannot be slot symmetries: Bianchi's cyclic
/// permutation has order 3 and Riemann's slot-symmetry group has order
/// 8, and 3 does not divide 8.
///
/// **`--bianchi HEAD` is deliberately explicit, never inferred.** The
/// first Bianchi identity is *not* a consequence of having Riemann's
/// slot symmetries -- a tensor can carry the pair antisymmetries and the
/// pair swap without satisfying the cyclic identity (that is exactly why
/// DESIGN-M4.md registers it "as an independent fact"). Inferring it
/// from the declared symmetry would be asserting a theorem the engine
/// cannot check, on the user's behalf. Requiring the flag also makes the
/// pedagogy visible: running the same sum with and without it shows,
/// concretely, that Bianchi is an extra axiom rather than bookkeeping.
fn run_simplify(mut args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let mut prelude_path = "prelude.od".to_string();
    let mut expr: Option<String> = None;
    let mut bianchi_heads: Vec<String> = Vec::new();
    let mut metric_heads: Vec<String> = Vec::new();
    let mut compatible_heads: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--prelude" => prelude_path = args.next().ok_or(CliError::Usage)?,
            "--bianchi" => bianchi_heads.push(args.next().ok_or(CliError::Usage)?),
            "--metric" => metric_heads.push(args.next().ok_or(CliError::Usage)?),
            // `nabla_a g_bc = 0` -- a property of the *connection* being
            // Levi-Civita, not of the metric's shape, so declared like
            // `--bianchi` rather than deduced. Distinct from `--metric`,
            // which contracts an *undifferentiated* metric away: a
            // differentiated one is a different object, and assuming it
            // vanishes is precisely this flag's job to state.
            "--metric-compatible" => compatible_heads.push(args.next().ok_or(CliError::Usage)?),
            _ => expr = Some(a),
        }
    }
    let expr = expr.ok_or(CliError::Usage)?;

    let prelude_src =
        std::fs::read_to_string(&prelude_path).map_err(|source| CliError::Io { path: prelude_path.clone(), source })?;
    let mut model = parser::parse_model(&prelude_src)?;

    let mut terms = parser::parse_polynomial(&expr, &mut model.registry)?;

    // Metric elimination runs *before* the e-graph, on each term's
    // contraction graph: it changes the term's factor count, which is
    // exactly the operation neither canonicalization nor the e-graph's
    // own rewrites can perform (DESIGN.md's reason for leaving this out
    // of Marco 1). Declared, never inferred -- see `--bianchi`.
    for name in &metric_heads {
        let head = model.registry.lookup_head(name)?;
        terms = terms.iter().map(|m| m.eliminate_metric(head, &model.registry)).collect::<Result<Vec<_>, _>>()?;
    }
    let terms = terms;

    let mut egraph = oderom_egraph::EGraph::new();
    let ids: smallvec::SmallVec<[oderom_egraph::EClassId; 4]> =
        terms.iter().map(|m| egraph.add_monomial(&model.registry, m)).collect();
    let root = egraph.add(oderom_egraph::ENode::Sum(ids));

    for name in &bianchi_heads {
        let head = model.registry.lookup_head(name)?;
        oderom_egraph::apply_bianchi(&mut egraph, &model.registry, head);
    }
    for name in &compatible_heads {
        let head = model.registry.lookup_head(name)?;
        oderom_egraph::apply_metric_compatibility(&mut egraph, &model.registry, head);
    }

    let start = Instant::now();
    let reduced = oderom_egraph::extract(&mut egraph, root);
    let collected = collect_like_terms(&reduced.terms, &model.registry)?;
    let elapsed = start.elapsed();

    if collected.is_empty() {
        println!("0");
    } else {
        let rendered: Vec<String> =
            collected.iter().map(|m| parser::format_monomial(m, &model.registry)).collect();
        println!("{}", rendered.join(" + "));
    }
    eprintln!(
        "({} termo{} -> {} termo{}, {:.3} ms)",
        terms.len(),
        if terms.len() == 1 { "" } else { "s" },
        collected.len(),
        if collected.len() == 1 { "" } else { "s" },
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Minimal number of transpositions realizing `perm`: `degree - #cycles`.
fn transposition_count(perm: &oderom_core::Perm) -> usize {
    let n = perm.len();
    let mut visited = vec![false; n];
    let mut cycles = 0;
    for start in 0..n {
        if visited[start] {
            continue;
        }
        cycles += 1;
        let mut cur = start;
        while !visited[cur] {
            visited[cur] = true;
            cur = perm.image(cur as u16) as usize;
        }
    }
    n - cycles
}
