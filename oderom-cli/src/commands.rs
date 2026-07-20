//! The five curvature subcommands (DESIGN-UI.md 6.4): `christoffel`,
//! `riemann`, `ricci`, `scalar`, `kretschmann`. Each loads a `.od` file
//! into a [`Model`], resolves which declared `metric`/`connection` to
//! use, computes the requested quantity via `oderom-components::curvature`
//! (unchanged from Marco 2 -- see DESIGN-UI.md 6.0, nothing here needed a
//! new symbolic-math capability), and renders it.
//!
//! Gamma resolution: an explicit `--connection NAME` always wins. Failing
//! that, if the file has any `metric` at all, one is chosen (the flag, or
//! the sole metric, or an explicit ambiguity error if there is more than
//! one and no flag) and Gamma always comes from `christoffel()`
//! (Levi-Civita) -- a `connection` declared alongside a `metric` is not
//! silently preferred. Only with zero metrics does a bare `connection` get
//! used by default. `scalar`/`kretschmann` need `g^ab` to invert/contract
//! and error cleanly (`NeedsMetric`) rather than compute a meaningless
//! number from a connection alone.

use crate::error::CliError;
use crate::model::Model;
use oderom_components::curvature::{
    christoffel, grid_to_component_tensor, kretschmann as kretschmann_of, lower_first_index,
    metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed,
};
use oderom_components::{classify_grid, classify_tensor, render_classes, Chart, ComponentTensor, Grid};
use oderom_core::{HeadId, Perm, Registry, Render, SignedPerm, SlotSig, Target, Variance};
use smallvec::SmallVec;
use std::collections::HashMap;

pub struct Args {
    file: String,
    metric: Option<String>,
    connection: Option<String>,
    target: Target,
    max_lines: usize,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, CliError> {
    let mut file = None;
    let mut metric = None;
    let mut connection = None;
    let mut target = Target::Unicode;
    let mut max_lines = 20usize;
    let mut args = args;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--metric" => metric = Some(args.next().ok_or(CliError::Usage)?),
            "--connection" => connection = Some(args.next().ok_or(CliError::Usage)?),
            "--target" => {
                target = match args.next().ok_or(CliError::Usage)?.as_str() {
                    "unicode" => Target::Unicode,
                    "latex" => Target::Latex,
                    "json" => Target::Json,
                    _ => return Err(CliError::Usage),
                };
            }
            "--max-lines" => {
                max_lines = args.next().ok_or(CliError::Usage)?.parse().map_err(|_| CliError::Usage)?;
            }
            _ if file.is_none() => file = Some(a),
            _ => return Err(CliError::Usage),
        }
    }
    Ok(Args { file: file.ok_or(CliError::Usage)?, metric, connection, target, max_lines })
}

fn load_model(args: &Args) -> Result<Model, CliError> {
    let src = std::fs::read_to_string(&args.file).map_err(|source| CliError::Io { path: args.file.clone(), source })?;
    crate::parser::parse_model(&src)
}

/// The flag's value if given, else the map's single entry, else an
/// explicit ambiguity error -- never a silent guess among several.
fn resolve_choice<'a, V>(
    map: &'a HashMap<String, V>,
    flag: &Option<String>,
    kind: &'static str,
) -> Result<Option<(&'a String, &'a V)>, CliError> {
    if let Some(name) = flag {
        return map
            .get_key_value(name)
            .map(Some)
            .ok_or_else(|| CliError::Parse(format!("no {kind} named `{name}` in this file")));
    }
    match map.len() {
        0 => Ok(None),
        1 => Ok(map.iter().next()),
        _ => {
            let mut names: Vec<&str> = map.keys().map(String::as_str).collect();
            names.sort_unstable();
            Err(CliError::AmbiguousChoice { kind, names: names.join(", ") })
        }
    }
}

struct MetricSource {
    chart: Chart,
    head: HeadId,
    tensor: ComponentTensor,
    ginv: Grid,
    gamma: Grid,
}

enum GammaSource {
    FromMetric(MetricSource),
    FromConnection { chart: Chart, gamma: Grid },
}

fn resolve_gamma_source(model: &Model, args: &Args) -> Result<GammaSource, CliError> {
    if let Some(name) = &args.connection {
        let (chart_name, gamma) = model
            .connections
            .get(name)
            .ok_or_else(|| CliError::Parse(format!("no connection named `{name}` in this file")))?;
        return Ok(build_from_connection(model, chart_name, gamma));
    }
    if !model.metrics.is_empty() || args.metric.is_some() {
        if let Some((_, (chart_name, head, tensor))) = resolve_choice(&model.metrics, &args.metric, "metric")? {
            return build_from_metric(model, chart_name, *head, tensor);
        }
    }
    if let Some((_, (chart_name, gamma))) = resolve_choice(&model.connections, &None, "connection")? {
        return Ok(build_from_connection(model, chart_name, gamma));
    }
    Err(CliError::NoMetricOrConnection)
}

fn build_from_metric(model: &Model, chart_name: &str, head: HeadId, tensor: &ComponentTensor) -> Result<GammaSource, CliError> {
    let chart = model.charts.get(chart_name).expect("chart name stored by parse_metric_decl always exists").clone();
    let ginv = metric_inverse_diagonal(&model.registry, &chart, tensor)?;
    let gamma = christoffel(&model.registry, &chart, tensor, &ginv)?;
    Ok(GammaSource::FromMetric(MetricSource { chart, head, tensor: tensor.clone(), ginv, gamma }))
}

fn build_from_connection(model: &Model, chart_name: &str, gamma: &Grid) -> GammaSource {
    let chart = model.charts.get(chart_name).expect("chart name stored by parse_connection_decl always exists").clone();
    GammaSource::FromConnection { chart, gamma: gamma.clone() }
}

fn require_metric(source: GammaSource, subcommand: &str) -> Result<MetricSource, CliError> {
    match source {
        GammaSource::FromMetric(m) => Ok(m),
        GammaSource::FromConnection { .. } => Err(CliError::NeedsMetric { name: subcommand.to_string() }),
    }
}

/// A rank-4 head with Riemann's slot symmetry, or a symmetric rank-2 head
/// (Ricci) -- declared internally to exploit `classify_tensor`'s elision,
/// never something the user writes: which components a curvature tensor
/// has is a mathematical fact of its rank and symmetry, not user data.
fn declare_internal_head(registry: &mut Registry, reuse_slots_from: HeadId, rank: usize, name: &str) -> Result<HeadId, CliError> {
    let bundle = registry.head(reuse_slots_from).slots[0].bundle;
    let dim = registry.head(reuse_slots_from).slots[0].dim;
    let co = SlotSig { bundle, variance: Variance::Co, dim };
    let slots: SmallVec<[SlotSig; 4]> = (0..rank).map(|_| co).collect();
    let gens = match rank {
        2 => vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)],
        4 => {
            let pair_swap = Perm::try_from_images(&[2, 3, 0, 1]).expect("hardcoded permutation of degree 4");
            vec![
                SignedPerm::new(Perm::transposition(4, 0, 1), -1),
                SignedPerm::new(Perm::transposition(4, 2, 3), -1),
                SignedPerm::new(pair_swap, 1),
            ]
        }
        _ => unreachable!("only rank 2 (Ricci) and rank 4 (Riemann) are used here"),
    };
    Ok(registry.declare_head(name, slots, gens)?)
}

pub fn christoffel_cmd(args: Args) -> Result<(), CliError> {
    let model = load_model(&args)?;
    let source = resolve_gamma_source(&model, &args)?;
    let gamma = match &source {
        GammaSource::FromMetric(m) => &m.gamma,
        GammaSource::FromConnection { gamma, .. } => gamma,
    };
    println!("{}", render_classes("Gamma", classify_grid(gamma), args.target, args.max_lines));
    Ok(())
}

pub fn riemann_cmd(args: Args) -> Result<(), CliError> {
    let mut model = load_model(&args)?;
    let source = resolve_gamma_source(&model, &args)?;
    match source {
        GammaSource::FromMetric(m) => {
            let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
            let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
            let riemann_head = declare_internal_head(&mut model.registry, m.head, 4, "__Riemann")?;
            let tensor = grid_to_component_tensor(&model.registry, riemann_head, &riem_cov);
            let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
            println!("{}", render_classes("R", classes, args.target, args.max_lines));
        }
        GammaSource::FromConnection { chart, gamma } => {
            let riem_mixed = riemann_mixed(&chart, &gamma);
            println!("{}", render_classes("R", classify_grid(&riem_mixed), args.target, args.max_lines));
        }
    }
    Ok(())
}

pub fn ricci_cmd(args: Args) -> Result<(), CliError> {
    let mut model = load_model(&args)?;
    let source = resolve_gamma_source(&model, &args)?;
    match source {
        GammaSource::FromMetric(m) => {
            let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
            let ricci = ricci_tensor(&m.chart, &riem_mixed);
            let ricci_head = declare_internal_head(&mut model.registry, m.head, 2, "__Ricci")?;
            let tensor = grid_to_component_tensor(&model.registry, ricci_head, &ricci);
            let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
            println!("{}", render_classes("Ricci", classes, args.target, args.max_lines));
        }
        GammaSource::FromConnection { chart, gamma } => {
            let riem_mixed = riemann_mixed(&chart, &gamma);
            let ricci = ricci_tensor(&chart, &riem_mixed);
            println!("{}", render_classes("Ricci", classify_grid(&ricci), args.target, args.max_lines));
        }
    }
    Ok(())
}

pub fn scalar_cmd(args: Args) -> Result<(), CliError> {
    let model = load_model(&args)?;
    let source = resolve_gamma_source(&model, &args)?;
    let m = require_metric(source, "scalar")?;
    let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
    let ricci = ricci_tensor(&m.chart, &riem_mixed);
    let r = ricci_scalar(&m.chart, &ricci, &m.ginv);
    println!("{}", r.render(args.target));
    Ok(())
}

pub fn kretschmann_cmd(args: Args) -> Result<(), CliError> {
    let model = load_model(&args)?;
    let source = resolve_gamma_source(&model, &args)?;
    let m = require_metric(source, "kretschmann")?;
    let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
    let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
    let k = kretschmann_of(&m.chart, &riem_cov, &m.ginv);
    println!("{}", k.render(args.target));
    Ok(())
}
