use oderom_cli::commands;
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
    let model = parser::parse_model(&prelude_src)?;

    // `canon` computes the canonical form of the contraction graph itself
    // (Marco 1.3); it does not run the separate geometric type judgment
    // (Marco 1.2, exercised directly by `oderom-types`'s own test suite).
    // Requiring dual variance on every contraction here would reject the
    // very examples Marco 1's acceptance table exercises, since `R` and
    // `g` are declared fully covariant by default (see `prelude.od`) and
    // Marco 1 has no index raising/lowering to reconcile that with.
    let monomial = parser::parse_monomial(&expr, &model.registry)?;

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
