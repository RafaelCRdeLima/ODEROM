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
//!
//! # Guardrail (measured, not guessed -- see DESIGN-M2.md's rational
//! normal form note)
//!
//! Every subcommand runs its whole computation on a background thread
//! under [`run_with_budget`], with a wall-clock `--timeout` (default
//! 30s) and stage names reported as progress on stderr as each stage
//! starts. If the timeout fires, the last stage name reported is exactly
//! what's in flight when the abort happens.
//!
//! A wall-clock timeout, not just a node-count ceiling, because a real
//! measurement (Reissner-Nordstrom's 3-term `f(r)`, `diagnostic_rn.rs`)
//! showed the failure mode is a *compute-time* explosion inside a single
//! `normalize()` call on a tree that never gets large (2889 nodes for
//! the full Kretschmann sum) -- a node-count check on the input would
//! not have caught it, since the input never crosses any reasonable
//! threshold. `--max-nodes` (default 20000) is still checked after every
//! discrete `Grid`-producing stage, because it protects against a
//! different, real failure mode (genuine tree blowup) that a pure
//! timeout wouldn't name as precisely.
//!
//! `--max-denominator-degree` (default 30) is the guardrail the node
//! count one provably can't be: `oderom_expr::denominator_degree`
//! (`rationalize()` then the standard recursive polynomial-degree
//! definition on the resulting denominator) grows exactly where the
//! blowup starts (measured on real Reissner-Nordstrom terms: 0 at 16
//! summed terms, 111 at 32) instead of staying flat like node count
//! does. It is *not* a cheap check, though -- it costs about as much as
//! `normalize()` itself, since it goes through the same machinery -- so
//! `kretschmann` only evaluates it at the same cadence as progress
//! reporting (`PROGRESS_STRIDE`), not every term; its value is a more
//! precise diagnostic when it fires, not a smaller time budget.
//!
//! `kretschmann` specifically does not call `curvature::kretschmann` as
//! one opaque call: it re-implements the same sum term-by-term here,
//! `normalize`-ing and checking the node budget incrementally, so a
//! timeout can report which of the (up to `dim^4`) terms it reached
//! before running out of time, and so a run that *would* finish just
//! keeps a smaller working set throughout instead of building all the
//! raw terms first.

use crate::error::CliError;
use crate::model::Model;
use oderom_components::curvature::{
    christoffel, grid_to_component_tensor, lower_first_index, metric_inverse_diagonal, raise_index, ricci_scalar,
    ricci_tensor, riemann_mixed,
};
use oderom_components::{classify_grid, classify_tensor, render_classes, Chart, ComponentTensor, Grid};
use oderom_core::{HeadId, Perm, Registry, Render, SignedPerm, SlotSig, Target, Variance};
use oderom_expr::{denominator_degree, normalize, Expr};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_MAX_NODES: usize = 20_000;
const DEFAULT_MAX_DENOMINATOR_DEGREE: i32 = 30;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Args {
    file: String,
    metric: Option<String>,
    connection: Option<String>,
    target: Target,
    max_lines: usize,
    max_nodes: usize,
    max_denominator_degree: i32,
    timeout: Duration,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, CliError> {
    let mut file = None;
    let mut metric = None;
    let mut connection = None;
    let mut target = Target::Unicode;
    let mut max_lines = 20usize;
    let mut max_nodes = DEFAULT_MAX_NODES;
    let mut max_denominator_degree = DEFAULT_MAX_DENOMINATOR_DEGREE;
    let mut timeout = DEFAULT_TIMEOUT;
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
            "--max-nodes" => {
                max_nodes = args.next().ok_or(CliError::Usage)?.parse().map_err(|_| CliError::Usage)?;
            }
            "--max-denominator-degree" => {
                max_denominator_degree = args.next().ok_or(CliError::Usage)?.parse().map_err(|_| CliError::Usage)?;
            }
            "--timeout" => {
                let secs: f64 = args.next().ok_or(CliError::Usage)?.parse().map_err(|_| CliError::Usage)?;
                timeout = Duration::from_secs_f64(secs);
            }
            _ if file.is_none() => file = Some(a),
            _ => return Err(CliError::Usage),
        }
    }
    Ok(Args {
        file: file.ok_or(CliError::Usage)?,
        metric,
        connection,
        target,
        max_lines,
        max_nodes,
        max_denominator_degree,
        timeout,
    })
}

fn load_model(args: &Args) -> Result<Model, CliError> {
    let src = std::fs::read_to_string(&args.file).map_err(|source| CliError::Io { path: args.file.clone(), source })?;
    crate::parser::parse_model(&src)
}

// ---------------------------------------------------------------------
// Guardrail: progress reporting + wall-clock budget for the whole
// subcommand, node-count checks at each discrete stage.
// ---------------------------------------------------------------------

/// Shared between the worker thread (which sets the current stage as it
/// goes) and the main thread (which reads it back only if the timeout
/// fires, for the diagnostic).
struct Progress {
    stage: Mutex<String>,
}

impl Progress {
    fn new() -> Arc<Self> {
        Arc::new(Progress { stage: Mutex::new("starting".to_string()) })
    }

    /// Records `stage` as current and echoes it to stderr -- so a
    /// long-but-finite run is never silent, even before any timeout is
    /// close to firing.
    fn set(&self, stage: impl Into<String>) {
        let stage = stage.into();
        eprintln!("{stage}...");
        // A poisoned lock here would mean the worker thread panicked
        // mid-update; the status string itself has no invariant that
        // panic could have broken, so recovering it is safe and more
        // useful than propagating the poison.
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = stage;
    }

    fn current(&self) -> String {
        self.stage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Checks `grid`'s total node count against `args.max_nodes` right after
/// a discrete computation stage produced it, naming `stage` in the error
/// if it's over -- catches genuine tree blowup at exactly the stage that
/// caused it, independent of the wall-clock budget below.
fn check_grid_budget(grid: &Grid, stage: &str, max_nodes: usize) -> Result<(), CliError> {
    let dim = grid.dim();
    let rank = grid.rank();
    let total = each_index_tuple(dim, rank).map(|idx| grid.get(&idx).node_count()).sum();
    if total > max_nodes {
        return Err(CliError::NodeLimitExceeded { stage: stage.to_string(), nodes: total, limit: max_nodes });
    }
    Ok(())
}

fn each_index_tuple(dim: usize, rank: usize) -> impl Iterator<Item = Vec<u8>> {
    let total = dim.pow(rank as u32);
    (0..total).map(move |mut n| {
        let mut tuple = vec![0u8; rank];
        for slot in tuple.iter_mut().rev() {
            *slot = (n % dim) as u8;
            n /= dim;
        }
        tuple
    })
}

/// Runs `f` on a background thread under a wall-clock budget: `f` reports
/// its own progress via the `Progress` handle it's given, and if
/// `timeout` elapses before `f` returns, this returns `CliError::Timeout`
/// naming the last stage `f` reported -- the computation itself is left
/// running in its detached thread (Rust has no safe way to force-stop
/// it), but the command returns promptly either way, which is the
/// user-visible thing that matters: never silent, never stuck.
fn run_with_budget<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce(&Progress) -> Result<T, CliError> + Send + 'static,
) -> Result<T, CliError> {
    let progress = Progress::new();
    let worker_progress = progress.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = f(&worker_progress);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err(CliError::Timeout { stage: progress.current(), timeout }),
    }
}

// ---------------------------------------------------------------------
// Gamma resolution (metric vs. connection)
// ---------------------------------------------------------------------

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

fn resolve_gamma_source(model: &Model, args: &Args, progress: &Progress) -> Result<GammaSource, CliError> {
    if let Some(name) = &args.connection {
        let (chart_name, gamma) = model
            .connections
            .get(name)
            .ok_or_else(|| CliError::Parse(format!("no connection named `{name}` in this file")))?;
        return Ok(build_from_connection(model, chart_name, gamma));
    }
    if !model.metrics.is_empty() || args.metric.is_some() {
        if let Some((_, (chart_name, head, tensor))) = resolve_choice(&model.metrics, &args.metric, "metric")? {
            return build_from_metric(model, chart_name, *head, tensor, args.max_nodes, progress);
        }
    }
    if let Some((_, (chart_name, gamma))) = resolve_choice(&model.connections, &None, "connection")? {
        return Ok(build_from_connection(model, chart_name, gamma));
    }
    Err(CliError::NoMetricOrConnection)
}

fn build_from_metric(
    model: &Model,
    chart_name: &str,
    head: HeadId,
    tensor: &ComponentTensor,
    max_nodes: usize,
    progress: &Progress,
) -> Result<GammaSource, CliError> {
    let chart = model.charts.get(chart_name).expect("chart name stored by parse_metric_decl always exists").clone();
    progress.set("inverting the metric");
    let ginv = metric_inverse_diagonal(&model.registry, &chart, tensor)?;
    progress.set("computing Christoffel symbols");
    let gamma = christoffel(&model.registry, &chart, tensor, &ginv)?;
    check_grid_budget(&gamma, "christoffel", max_nodes)?;
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

// ---------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------

pub fn christoffel_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |progress| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, &args, progress)?;
        let gamma = match &source {
            GammaSource::FromMetric(m) => &m.gamma,
            GammaSource::FromConnection { gamma, .. } => gamma,
        };
        progress.set("rendering");
        Ok(render_classes("Gamma", classify_grid(gamma), args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

pub fn riemann_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |progress| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, &args, progress)?;
        let text = match source {
            GammaSource::FromMetric(m) => {
                progress.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                progress.set("lowering the first index");
                let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
                check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
                let riemann_head = declare_internal_head(&mut model.registry, m.head, 4, "__Riemann")?;
                let tensor = grid_to_component_tensor(&model.registry, riemann_head, &riem_cov);
                let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                render_classes("R", classes, args.target, args.max_lines)
            }
            GammaSource::FromConnection { chart, gamma } => {
                progress.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&chart, &gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                render_classes("R", classify_grid(&riem_mixed), args.target, args.max_lines)
            }
        };
        Ok(text)
    })?;
    println!("{text}");
    Ok(())
}

pub fn ricci_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |progress| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, &args, progress)?;
        let text = match source {
            GammaSource::FromMetric(m) => {
                progress.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                progress.set("contracting to the Ricci tensor");
                let ricci = ricci_tensor(&m.chart, &riem_mixed);
                check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                let ricci_head = declare_internal_head(&mut model.registry, m.head, 2, "__Ricci")?;
                let tensor = grid_to_component_tensor(&model.registry, ricci_head, &ricci);
                let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                render_classes("Ricci", classes, args.target, args.max_lines)
            }
            GammaSource::FromConnection { chart, gamma } => {
                progress.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&chart, &gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                progress.set("contracting to the Ricci tensor");
                let ricci = ricci_tensor(&chart, &riem_mixed);
                check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                render_classes("Ricci", classify_grid(&ricci), args.target, args.max_lines)
            }
        };
        Ok(text)
    })?;
    println!("{text}");
    Ok(())
}

pub fn scalar_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |progress| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, &args, progress)?;
        let m = require_metric(source, "scalar")?;
        progress.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        progress.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        progress.set("contracting to the Ricci scalar");
        let r = ricci_scalar(&m.chart, &ricci, &m.ginv);
        Ok(r.render(args.target))
    })?;
    println!("{text}");
    Ok(())
}

/// Unlike the other four, this does not call `curvature::kretschmann` as
/// one opaque call -- see the module docs: it accumulates the same
/// `dim^4`-term sum here, `normalize`-ing and checking the node budget
/// after every term, and reporting progress every `PROGRESS_STRIDE`
/// terms, so a timeout can say which term it reached.
const PROGRESS_STRIDE: usize = 16;

pub fn kretschmann_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |progress| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, &args, progress)?;
        let m = require_metric(source, "kretschmann")?;

        progress.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;

        progress.set("lowering the first index");
        let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
        check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;

        progress.set("raising indices (1/4)");
        let raised0 = raise_index(&m.chart, &riem_cov, &m.ginv, 0);
        check_grid_budget(&raised0, "raise_index(0)", args.max_nodes)?;
        progress.set("raising indices (2/4)");
        let raised1 = raise_index(&m.chart, &raised0, &m.ginv, 1);
        check_grid_budget(&raised1, "raise_index(1)", args.max_nodes)?;
        progress.set("raising indices (3/4)");
        let raised2 = raise_index(&m.chart, &raised1, &m.ginv, 2);
        check_grid_budget(&raised2, "raise_index(2)", args.max_nodes)?;
        progress.set("raising indices (4/4)");
        let riemann_contra = raise_index(&m.chart, &raised2, &m.ginv, 3);
        check_grid_budget(&riemann_contra, "raise_index(3)", args.max_nodes)?;

        let dim = m.chart.dim();
        let total_terms = dim.pow(4);
        let mut sum = Expr::zero();
        for (index, idx) in each_index_tuple(dim, 4).enumerate() {
            let term = riem_cov.get(&idx) * riemann_contra.get(&idx);
            sum = normalize(&(sum + term));
            let nodes = sum.node_count();
            if nodes > args.max_nodes {
                return Err(CliError::NodeLimitExceeded {
                    stage: format!("kretschmann sum (term {}/{total_terms})", index + 1),
                    nodes,
                    limit: args.max_nodes,
                });
            }
            if index % PROGRESS_STRIDE == 0 || index + 1 == total_terms {
                // denominator_degree costs about as much as normalize()
                // itself (both go through the same rationalize/normalize
                // machinery -- measured, not assumed: diagnostic_rn.rs),
                // so it is not a cheap early check and is only worth
                // running at the same cadence as progress reporting, not
                // every term. Its value is a more precise diagnostic when
                // it does fire, not a smaller time budget.
                let degree = denominator_degree(&sum);
                if degree > args.max_denominator_degree {
                    return Err(CliError::DenominatorDegreeExceeded {
                        stage: format!("kretschmann sum (term {}/{total_terms})", index + 1),
                        degree,
                        limit: args.max_denominator_degree,
                    });
                }
                progress.set(format!(
                    "summing Kretschmann terms ({}/{total_terms}, {nodes} nodes, denominator degree {degree})",
                    index + 1
                ));
            }
        }
        Ok(sum.render(args.target))
    })?;
    println!("{text}");
    Ok(())
}
