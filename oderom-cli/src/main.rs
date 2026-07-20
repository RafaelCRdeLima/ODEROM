mod commands;
mod error;
mod expr_parser;
mod index_resolve;
mod model;
mod parser;

use error::CliError;
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
        _ => Err(CliError::Usage),
    }
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
