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
use crate::parser::CommandName;
use oderom_components::curvature::{
    accel_equations_checkpointed, christoffel_checkpointed, einstein_tensor, einstein_tensor_checkpointed, gauss_bonnet,
    geodesic_equations_checkpointed, grid_to_component_tensor, kretschmann, kretschmann_checkpointed, lower_first_index,
    lower_first_index_checkpointed, metric_inverse, raise_index, ricci_scalar, ricci_squared, ricci_squared_checkpointed,
    ricci_tensor, ricci_tensor_checkpointed, riemann_mixed, riemann_mixed_checkpointed, weyl_squared, weyl_squared_checkpointed,
    weyl_tensor, weyl_tensor_checkpointed,
};
use oderom_components::{
    classify_grid, classify_tensor, render_classes, render_geodesic, render_geodesic_solved, Chart, ComponentClass, ComponentTensor,
    Grid, CHRISTOFFEL_VARIANCE, RICCI_VARIANCE, RIEMANN_COV_VARIANCE, RIEMANN_MIXED_VARIANCE,
};
use oderom_core::{HeadId, Perm, Registry, Render, SignedPerm, SlotSig, Target, Variance};
use oderom_expr::export::{apply_renames, build_rename_map, collect_names, comment_line, MATHEMATICA_RESERVED, SYMPY_RESERVED};
use oderom_expr::{denominator_degree, normalize, Expr};
use smallvec::SmallVec;
use std::collections::{BTreeSet, HashMap};
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
    /// The affine parameter's name -- `geodesic` only, via `--param`;
    /// `None` for the other five subcommands, which never read this
    /// field. Required by `geodesic_cmd` itself (a missing `--param` is
    /// `CliError::Usage`, checked there rather than by adding a second,
    /// command-aware `parse_args` just for this one flag.
    param: Option<String>,
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
    let mut param = None;
    let mut args = args;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--metric" => metric = Some(args.next().ok_or(CliError::Usage)?),
            "--connection" => connection = Some(args.next().ok_or(CliError::Usage)?),
            "--param" => param = Some(args.next().ok_or(CliError::Usage)?),
            "--target" => {
                // `mathematica`/`sympy` render with the same raw syntax
                // `export` produces, but WITHOUT `export`'s own
                // reserved-word detection/renaming or SymPy `symbols(...)`
                // header (Rodada Exportação) -- a quick one-off look at
                // the translated syntax, not a guaranteed-pasteable
                // result the way `oderom export TARGET COMMAND FILE` is;
                // reach for `export` instead when the output needs to
                // actually run in the target tool unmodified.
                target = match args.next().ok_or(CliError::Usage)?.as_str() {
                    "unicode" => Target::Unicode,
                    "latex" => Target::Latex,
                    "json" => Target::Json,
                    "mathematica" => Target::Mathematica,
                    "sympy" => Target::Sympy,
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
        param,
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
/// goes) and whoever's watching -- the CLI's main thread, which reads
/// `current()` back only if the timeout fires; `oderom-session`, which
/// reads `used()` after a query resolves to know which declared names
/// it depended on (DESIGN-UI-SESSION.md section 2); the REPL
/// (`oderom-repl`), which calls `cancel()` from its Ctrl+C handler while
/// a background thread's `christoffel_checkpointed`/`riemann_mixed_checkpointed`/
/// etc. calls poll `is_cancelled()` once per component. Named
/// `ExecutionContext`, not `Progress`, since progress reporting was
/// always only one of its jobs.
pub struct ExecutionContext {
    stage: Mutex<String>,
    used: Mutex<BTreeSet<String>>,
    cancel_token: oderom_core::CancelToken,
}

impl ExecutionContext {
    pub fn new() -> Arc<Self> {
        Arc::new(ExecutionContext {
            stage: Mutex::new("starting".to_string()),
            used: Mutex::new(BTreeSet::new()),
            cancel_token: oderom_core::CancelToken::new(),
        })
    }

    /// Requests cancellation -- checked by every `*_checkpointed` stage
    /// function's checkpoint, once per independent component (measured
    /// worst-case latency, Reissner-Nordstrom, release build:
    /// `oderom-components/tests/diagnostic_cancel_latency.rs`), *and*
    /// by `oderom_expr::cancel`'s deep, per-loop checks inside
    /// `normalize()`/`poly_gcd`/the subresultant PRS (armed via
    /// `token()` below) -- the between-component checkpoint alone left
    /// a real gap: a metric whose `g_tt`/`g_rr` are not reciprocal can
    /// make a *single* component's `normalize()` call itself run away,
    /// which the between-component check can never interrupt because
    /// that call never returns to it.
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// The same flag `cancel()`/`is_cancelled()` above read and write,
    /// handed to `oderom_expr::run_cancellable` so its thread-local
    /// scope and this context's between-component checks are watching
    /// one shared flag, not two independent ones that could disagree.
    pub fn token(&self) -> oderom_core::CancelToken {
        self.cancel_token.clone()
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

    /// The channel Etapa 3b's cancellation work (DESIGN-NOTEBOOK.md)
    /// deliberately keeps open for the *next* round (per-stage progress
    /// display) without implementing it yet: whatever holds this
    /// `ExecutionContext` alive for cancellation purposes (`Notebook`'s
    /// own `running` map) already has everything needed to read the
    /// current stage too -- no new plumbing, just a DTO field and a
    /// frontend to add later. `pub` since that future caller lives
    /// outside this crate; nothing here reads it yet.
    pub fn current(&self) -> String {
        self.stage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Records that a declared name (a chart, metric, or connection) was
    /// consulted while resolving/computing a query -- called at every
    /// point `resolve_gamma_source`/`build_from_metric`/
    /// `build_from_connection` look a name up by string, chart included
    /// (a real gap in an earlier version of this design: only the
    /// metric/connection name was recorded, so editing just the chart a
    /// metric refers to invalidated nothing).
    pub fn record_use(&self, name: &str) {
        self.used.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string());
    }

    /// The full set of names recorded via `record_use` since this
    /// context was created.
    pub fn used(&self) -> BTreeSet<String> {
        self.used.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Checks `grid`'s total node count against `args.max_nodes` right after
/// a discrete computation stage produced it, naming `stage` in the error
/// if it's over -- catches genuine tree blowup at exactly the stage that
/// caused it, independent of the wall-clock budget below.
pub fn check_grid_budget(grid: &Grid, stage: &str, max_nodes: usize) -> Result<(), CliError> {
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
/// its own progress via the `ExecutionContext` handle it's given, and if
/// `timeout` elapses before `f` returns, this returns `CliError::Timeout`
/// naming the last stage `f` reported -- the computation itself is left
/// running in its detached thread (Rust has no safe way to force-stop
/// it), but the command returns promptly either way, which is the
/// user-visible thing that matters: never silent, never stuck.
fn run_with_budget<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce(&ExecutionContext) -> Result<T, CliError> + Send + 'static,
) -> Result<T, CliError> {
    let ctx = ExecutionContext::new();
    let worker_ctx = ctx.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = f(&worker_ctx);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err(CliError::Timeout { stage: ctx.current(), timeout }),
    }
}

// ---------------------------------------------------------------------
// Gamma resolution (metric vs. connection)
// ---------------------------------------------------------------------

/// The flag's value if given, else the map's single entry, else an
/// explicit ambiguity error -- never a silent guess among several.
pub fn resolve_choice<'a, V>(
    map: &'a HashMap<String, V>,
    flag: &Option<String>,
    kind: &'static str,
) -> Result<Option<(&'a String, &'a V)>, CliError> {
    if let Some(name) = flag {
        return map
            .get_key_value(name)
            .map(Some)
            .ok_or_else(|| CliError::Parse { message: format!("no {kind} named `{name}` in this file"), position: None });
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

pub struct MetricSource {
    pub chart: Chart,
    pub head: HeadId,
    pub tensor: ComponentTensor,
    pub ginv: Grid,
    pub gamma: Grid,
}

pub enum GammaSource {
    FromMetric(MetricSource),
    FromConnection { chart: Chart, gamma: Grid },
}

/// `connection`/`metric` are `--connection`/`--metric` for the CLI, or a
/// `Query::Command`'s `target` for a worksheet entry -- same resolution
/// rule either way (DESIGN-UI-SESSION.md section 2), no `Args` needed
/// here at all, so this is reusable by both without either depending on
/// the other's argument-parsing type.
pub fn resolve_gamma_source(
    model: &Model,
    connection: Option<&str>,
    metric: Option<&str>,
    max_nodes: usize,
    ctx: &ExecutionContext,
) -> Result<GammaSource, CliError> {
    if let Some(name) = connection {
        let (chart_name, gamma) = model
            .connections
            .get(name)
            .ok_or_else(|| CliError::Parse { message: format!("no connection named `{name}` in this file"), position: None })?;
        ctx.record_use(name);
        return Ok(build_from_connection(model, chart_name, gamma, ctx));
    }
    if !model.metrics.is_empty() || metric.is_some() {
        let metric_owned = metric.map(str::to_string);
        if let Some((resolved_name, (chart_name, head, tensor))) = resolve_choice(&model.metrics, &metric_owned, "metric")? {
            ctx.record_use(resolved_name);
            return build_from_metric(model, chart_name, *head, tensor, max_nodes, ctx);
        }
    }
    if let Some((resolved_name, (chart_name, gamma))) = resolve_choice(&model.connections, &None, "connection")? {
        ctx.record_use(resolved_name);
        return Ok(build_from_connection(model, chart_name, gamma, ctx));
    }
    Err(CliError::NoMetricOrConnection)
}

fn build_from_metric(
    model: &Model,
    chart_name: &str,
    head: HeadId,
    tensor: &ComponentTensor,
    max_nodes: usize,
    ctx: &ExecutionContext,
) -> Result<GammaSource, CliError> {
    ctx.record_use(chart_name);
    let chart = model.charts.get(chart_name).expect("chart name stored by parse_metric_decl always exists").clone();
    ctx.set("inverting the metric");
    let ginv = metric_inverse(&model.registry, &chart, tensor)?;
    ctx.set("computing Christoffel symbols");
    let gamma = christoffel_checkpointed(&model.registry, &chart, tensor, &ginv, &mut || ctx.is_cancelled())?;
    check_grid_budget(&gamma, "christoffel", max_nodes)?;
    Ok(GammaSource::FromMetric(MetricSource { chart, head, tensor: tensor.clone(), ginv, gamma }))
}

fn build_from_connection(model: &Model, chart_name: &str, gamma: &Grid, ctx: &ExecutionContext) -> GammaSource {
    ctx.record_use(chart_name);
    let chart = model.charts.get(chart_name).expect("chart name stored by parse_connection_decl always exists").clone();
    GammaSource::FromConnection { chart, gamma: gamma.clone() }
}

pub fn require_metric(source: GammaSource, subcommand: &str) -> Result<MetricSource, CliError> {
    match source {
        GammaSource::FromMetric(m) => Ok(m),
        GammaSource::FromConnection { .. } => Err(CliError::NeedsMetric { name: subcommand.to_string() }),
    }
}

/// `geodesic`'s mandatory collision rule (Marco 6 step 4, round B): the
/// affine parameter's name must not equal a chart coordinate, nor a free
/// variable already used in the metric/connection in scope (`M`, `Q`,
/// ...) -- either would make `x(param)` and a bare coordinate/parameter
/// with the same spelling genuinely ambiguous in the assembled equation,
/// not just visually confusing. Refused outright, never silently
/// resolved one way.
///
/// The metric case checks the METRIC's own raw components (`m.tensor`),
/// not its derived Christoffel grid: differentiating to build Gamma can
/// algebraically cancel a variable that the metric itself still
/// genuinely depends on (e.g. a term appearing in `g_tt` but not in any
/// of its derivatives), so checking Gamma alone could silently miss a
/// real collision. The connection-only case has no separate metric to
/// prefer, so it checks the connection's own raw Gamma grid instead --
/// the only tensor that exists in that case.
pub fn check_affine_parameter_collision(
    model: &Model,
    chart: &Chart,
    source: &GammaSource,
    param: &str,
    command_word: &str,
) -> Result<(), CliError> {
    if chart.coords.iter().any(|c| c == param) {
        return Err(CliError::Parse {
            message: format!("affine parameter `{param}` collides with chart coordinate `{param}` -- {command_word} needs a distinct name"),
            position: None,
        });
    }
    let mut used = std::collections::HashSet::new();
    match source {
        GammaSource::FromMetric(m) => {
            for idx in each_index_tuple(chart.dim(), 2) {
                if let Ok(expr) = m.tensor.get(&model.registry, &idx) {
                    used.extend(oderom_expr::free_vars(&expr));
                }
            }
        }
        GammaSource::FromConnection { gamma, .. } => {
            for idx in each_index_tuple(chart.dim(), 3) {
                used.extend(oderom_expr::free_vars(&gamma.get(&idx)));
            }
        }
    }
    if used.contains(param) {
        return Err(CliError::Parse {
            message: format!(
                "affine parameter `{param}` collides with a free variable already used in this metric/connection -- {command_word} needs a distinct name"
            ),
            position: None,
        });
    }
    Ok(())
}

/// A rank-4 head with Riemann's slot symmetry, or a symmetric rank-2 head
/// (Ricci) -- declared internally to exploit `classify_tensor`'s elision,
/// never something the user writes: which components a curvature tensor
/// has is a mathematical fact of its rank and symmetry, not user data.
pub fn declare_internal_head(registry: &mut Registry, reuse_slots_from: HeadId, rank: usize, name: &str) -> Result<HeadId, CliError> {
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
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let (chart, gamma) = match &source {
            GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
            GammaSource::FromConnection { chart, gamma } => (chart, gamma),
        };
        ctx.set("rendering");
        Ok(render_classes("Gamma", classify_grid(gamma), chart, &CHRISTOFFEL_VARIANCE, args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

pub fn riemann_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let text = match source {
            GammaSource::FromMetric(m) => {
                ctx.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                ctx.set("lowering the first index");
                let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
                check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
                let riemann_head = declare_internal_head(&mut model.registry, m.head, 4, "__Riemann")?;
                let tensor = grid_to_component_tensor(&model.registry, riemann_head, &riem_cov);
                let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                render_classes("R", classes, &m.chart, &RIEMANN_COV_VARIANCE, args.target, args.max_lines)
            }
            GammaSource::FromConnection { chart, gamma } => {
                ctx.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&chart, &gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                render_classes("R", classify_grid(&riem_mixed), &chart, &RIEMANN_MIXED_VARIANCE, args.target, args.max_lines)
            }
        };
        Ok(text)
    })?;
    println!("{text}");
    Ok(())
}

pub fn ricci_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let text = match source {
            GammaSource::FromMetric(m) => {
                ctx.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                ctx.set("contracting to the Ricci tensor");
                let ricci = ricci_tensor(&m.chart, &riem_mixed);
                check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                let ricci_head = declare_internal_head(&mut model.registry, m.head, 2, "__Ricci")?;
                let tensor = grid_to_component_tensor(&model.registry, ricci_head, &ricci);
                let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                render_classes("Ricci", classes, &m.chart, &RICCI_VARIANCE, args.target, args.max_lines)
            }
            GammaSource::FromConnection { chart, gamma } => {
                ctx.set("computing the Riemann tensor (mixed)");
                let riem_mixed = riemann_mixed(&chart, &gamma);
                check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                ctx.set("contracting to the Ricci tensor");
                let ricci = ricci_tensor(&chart, &riem_mixed);
                check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                render_classes("Ricci", classify_grid(&ricci), &chart, &RICCI_VARIANCE, args.target, args.max_lines)
            }
        };
        Ok(text)
    })?;
    println!("{text}");
    Ok(())
}

/// `G_ab = R_ab - (1/2) g_ab R` (Marco 6 step 5) -- always fully
/// covariant, always needs a real metric (unlike `christoffel`/
/// `riemann`/`ricci`, which all also work from a bare connection): `g_ab`
/// appears literally in the formula, and `R` itself already needs `g^bd`
/// to contract (`scalar`'s own reasoning, reused verbatim -- see
/// `require_metric`'s call below, same pattern as `scalar_cmd`/
/// `kretschmann_cmd`).
pub fn einstein_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "einstein")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting to the Ricci scalar");
        let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
        ctx.set("assembling the Einstein tensor");
        let einstein = einstein_tensor(&model.registry, &m.chart, &m.tensor, &ricci, &scalar);
        check_grid_budget(&einstein, "einstein_tensor", args.max_nodes)?;
        let einstein_head = declare_internal_head(&mut model.registry, m.head, 2, "__Einstein")?;
        let tensor = grid_to_component_tensor(&model.registry, einstein_head, &einstein);
        let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
        Ok(render_classes("G", classes, &m.chart, &RICCI_VARIANCE, args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

/// `R_ab R^ab` (Marco 6 step 6) -- a bare scalar, same shape as `scalar`/
/// `kretschmann`; needs a real metric to raise both of Ricci's indices.
pub fn riccisquare_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "riccisquare")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting Ricci with itself");
        let r = ricci_squared(&m.chart, &ricci, &m.ginv);
        Ok(r.render(args.target))
    })?;
    println!("{text}");
    Ok(())
}

/// `R_abcd R^abcd - 4 R_ab R^ab + R^2` (Marco 6 step 6, the Gauss-Bonnet
/// density) -- pure recombination of `kretschmann`/[`riccisquare_cmd`]'s
/// own scalar/`scalar` itself; needs a real metric only because those
/// three do.
pub fn gaussbonnet_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "gaussbonnet")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("lowering the first index");
        let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
        check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting to the Ricci scalar");
        let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
        ctx.set("computing the Kretschmann scalar");
        let k = kretschmann(&m.chart, &riem_cov, &m.ginv);
        ctx.set("contracting Ricci with itself");
        let rsq = ricci_squared(&m.chart, &ricci, &m.ginv);
        ctx.set("assembling the Gauss-Bonnet density");
        Ok(gauss_bonnet(&k, &rsq, &scalar).render(args.target))
    })?;
    println!("{text}");
    Ok(())
}

pub fn scalar_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "scalar")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting to the Ricci scalar");
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
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "kretschmann")?;

        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;

        ctx.set("lowering the first index");
        let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
        check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;

        ctx.set("raising indices (1/4)");
        let raised0 = raise_index(&m.chart, &riem_cov, &m.ginv, 0);
        check_grid_budget(&raised0, "raise_index(0)", args.max_nodes)?;
        ctx.set("raising indices (2/4)");
        let raised1 = raise_index(&m.chart, &raised0, &m.ginv, 1);
        check_grid_budget(&raised1, "raise_index(1)", args.max_nodes)?;
        ctx.set("raising indices (3/4)");
        let raised2 = raise_index(&m.chart, &raised1, &m.ginv, 2);
        check_grid_budget(&raised2, "raise_index(2)", args.max_nodes)?;
        ctx.set("raising indices (4/4)");
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
                ctx.set(format!(
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

/// The Weyl (conformal) tensor `C_abcd`, fully covariant (Marco 6 step
/// 6) -- a tensor-listing query like `riemann`/`ricci`, rendered the
/// same way, but needing a real metric (`g_ab`/`R` appear literally in
/// the formula) the way `scalar`/`kretschmann` do, unlike
/// `christoffel`/`riemann`/`ricci`. Refuses in exactly 2 dimensions --
/// `weyl_tensor` itself returns `ComponentError::WeylUndefinedInDimension2`,
/// converted to `CliError` via the same `#[from]` every other
/// `ComponentError` already uses, so no special handling is needed here
/// beyond the ordinary `?`.
pub fn weyl_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let mut model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "weyl")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("lowering the first index");
        let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
        check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting to the Ricci scalar");
        let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
        ctx.set("assembling the Weyl tensor");
        let weyl = weyl_tensor(&model.registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar)?;
        check_grid_budget(&weyl, "weyl_tensor", args.max_nodes)?;
        let weyl_head = declare_internal_head(&mut model.registry, m.head, 4, "__Weyl")?;
        let tensor = grid_to_component_tensor(&model.registry, weyl_head, &weyl);
        let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
        Ok(render_classes("C", classes, &m.chart, &RIEMANN_COV_VARIANCE, args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

/// `C_abcd C^abcd` (Marco 6 step 6) -- a bare scalar like `riccisquare`,
/// inheriting [`weyl_cmd`]'s own dimension-2 refusal (`weyl_squared`
/// computes the Weyl tensor first internally, propagating the same
/// error).
pub fn weylsquare_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let m = require_metric(source, "weylsquare")?;
        ctx.set("computing the Riemann tensor (mixed)");
        let riem_mixed = riemann_mixed(&m.chart, &m.gamma);
        check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
        ctx.set("lowering the first index");
        let riem_cov = lower_first_index(&model.registry, &m.chart, &riem_mixed, &m.tensor)?;
        check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
        ctx.set("contracting to the Ricci tensor");
        let ricci = ricci_tensor(&m.chart, &riem_mixed);
        check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
        ctx.set("contracting to the Ricci scalar");
        let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
        ctx.set("assembling and contracting the Weyl tensor");
        let wsq = weyl_squared(&model.registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &m.ginv)?;
        Ok(wsq.render(args.target))
    })?;
    println!("{text}");
    Ok(())
}

/// The geodesic equation, one closed-form equation per coordinate (Marco
/// 6 step 4, round B) -- writes the equations, does not solve them. Works
/// from a bare connection just as well as a metric (see `resolve_gamma_source`'s
/// own doc comment): unlike `scalar`/`kretschmann`, this only ever needs
/// `(chart, gamma)`, never `ginv`, so `require_metric` is never called
/// here at all.
pub fn geodesic_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let param = args.param.clone().ok_or(CliError::Usage)?;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let (chart, gamma) = match &source {
            GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
            GammaSource::FromConnection { chart, gamma } => (chart, gamma),
        };
        check_affine_parameter_collision(&model, chart, &source, &param, "geodesic")?;
        ctx.set("assembling the geodesic equations");
        let equations = geodesic_equations_checkpointed(chart, gamma, &param, &mut || ctx.is_cancelled())?;
        check_grid_budget_for_equations(&equations, "geodesic", args.max_nodes)?;
        ctx.set("rendering");
        Ok(render_geodesic("geodesic", &equations, &param, args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

/// Same node-count guardrail `check_grid_budget` applies to a `Grid`,
/// for a flat list of equations instead -- `geodesic_equations_checkpointed`
/// returns `Vec<Expr>`, not a `Grid`, so there is no shared indexing
/// scheme to reuse `each_index_tuple` for.
fn check_grid_budget_for_equations(equations: &[Expr], stage: &str, max_nodes: usize) -> Result<(), CliError> {
    let total: usize = equations.iter().map(Expr::node_count).sum();
    if total > max_nodes {
        return Err(CliError::NodeLimitExceeded { stage: stage.to_string(), nodes: total, limit: max_nodes });
    }
    Ok(())
}

/// The geodesic equation solved for each coordinate's own second
/// derivative, `x_a'' = f_a(x, x')` (Marco 6 step 4, round C) -- the
/// form a numerical integrator consumes directly. A separate command
/// from `geodesic` (unchanged, still the canonical `= 0` form), sharing
/// its exact same parameter syntax and collision rule -- see
/// `check_affine_parameter_collision`'s own doc comment -- and, like
/// `geodesic`, working from a bare `connection` just as well as a
/// metric, since it only ever needs `(chart, gamma)`.
pub fn accel_cmd(args: Args) -> Result<(), CliError> {
    let timeout = args.timeout;
    let param = args.param.clone().ok_or(CliError::Usage)?;
    let text = run_with_budget(timeout, move |ctx| {
        let model = load_model(&args)?;
        let source = resolve_gamma_source(&model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
        let (chart, gamma) = match &source {
            GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
            GammaSource::FromConnection { chart, gamma } => (chart, gamma),
        };
        check_affine_parameter_collision(&model, chart, &source, &param, "accel")?;
        ctx.set("assembling the geodesic equations");
        let solved = accel_equations_checkpointed(chart, gamma, &param, &mut || ctx.is_cancelled())?;
        check_grid_budget_for_equations(&solved, "accel", args.max_nodes)?;
        ctx.set("rendering");
        Ok(render_geodesic_solved("accel", chart, &solved, &param, args.target, args.max_lines))
    })?;
    println!("{text}");
    Ok(())
}

// ---------------------------------------------------------------------
// `export` (Rodada Exportação): translate a query's already-computed
// result into Mathematica or SymPy source instead of this project's own
// Unicode/LaTeX rendering, so a user can keep working symbolically in
// that tool. This is the CLI's own version of the same split
// `oderom-session::run::render_export` implements for the notebook --
// duplicated here rather than shared, the same way this file's own
// per-command dispatch (`riemann_cmd`, `ricci_cmd`, ...) already exists
// independently of `oderom-session::run::run_command_query`'s own
// separate copy (that module's own doc comment: "the only thing
// genuinely new there is wrapping the expensive stages in the compute
// cache" -- this file has no cache to begin with, so there is nothing
// to share beyond the lower-level pieces both already call,
// `resolve_gamma_source` and the rest of this section above).
// ---------------------------------------------------------------------

/// One query's result, computed but not yet rendered to any particular
/// target -- the CLI's own, intentionally small version of
/// `oderom-session::run::Rendered` (this binary renders once and exits,
/// never switches target on the same value the way the notebook's
/// cache-backed version can).
enum ExportSource {
    Classes { label: &'static str, classes: Vec<ComponentClass>, chart: Chart, variance: Vec<Variance> },
    Scalar(Expr),
    Equations { param: String, equations: Vec<Expr> },
    AccelEquations { chart: Chart, param: String, rhs: Vec<Expr> },
}

/// `oderom export TARGET COMMAND FILE [flags]` -- `TARGET` is
/// `mathematica`/`sympy`; `COMMAND` is any of the twelve query keywords
/// the notebook's own `Query` grammar recognizes, validated the same
/// way (`CommandName::from_str`), so an unknown command word gets the
/// identical error message either surface would give. `args` is
/// otherwise ordinary `Args` (`--metric`/`--connection`/`--param`/
/// `--max-nodes`/`--max-lines`(ignored, see below)/... all parse
/// unchanged) -- `--target unicode|latex|json` is accepted by
/// `parse_args` but never consulted here: the export target IS this
/// subcommand's own target, there is no second, separate choice to
/// make, and `--max-lines` never truncates export output (see
/// `render_export_text`'s own doc comment) -- both are simply ignored,
/// not rejected, so `export`'s own flag set stays a subset of every
/// other subcommand's, nothing new to document twice.
pub fn export_cmd(export_target: String, command_word: String, args: Args) -> Result<(), CliError> {
    let target = match export_target.as_str() {
        "mathematica" => Target::Mathematica,
        "sympy" => Target::Sympy,
        other => {
            return Err(CliError::Parse {
                message: format!("unknown export target `{other}` (expected one of: mathematica, sympy)"),
                position: None,
            })
        }
    };
    let Some(name) = CommandName::from_str(&command_word) else {
        return Err(CliError::Parse {
            message: format!(
                "unknown command `{command_word}` (expected one of: christoffel, riemann, ricci, scalar, kretschmann, geodesic, accel, einstein, riccisquare, gaussbonnet, weyl, weylsquare)"
            ),
            position: None,
        });
    };
    if matches!(name, CommandName::Geodesic | CommandName::Accel) && args.param.is_none() {
        return Err(CliError::Usage);
    }

    let timeout = args.timeout;
    let text = run_with_budget(timeout, move |ctx| {
        let mut model = load_model(&args)?;
        let source = compute_export_source(name, &mut model, &args, ctx)?;
        ctx.set("rendering");
        Ok(render_export_text(source, target))
    })?;
    println!("{text}");
    Ok(())
}

/// Computes exactly what the matching `_cmd` function above computes
/// for `name`, stopping one step short of the final `render_classes`/
/// `.render(...)` call that function makes -- see `ExportSource`'s own
/// doc comment for why this is a separate, CLI-local copy rather than a
/// shared function. Uses the `_checkpointed` stage functions throughout
/// (unlike most of the `_cmd` functions above, which mostly rely on
/// `run_with_budget`'s own wall-clock timeout instead), matching
/// `oderom-session::run::run_command_query`'s own choice -- export's
/// computation can be just as large as any other query's, so it
/// deserves the same fine-grained cancellation.
fn compute_export_source(name: CommandName, model: &mut Model, args: &Args, ctx: &ExecutionContext) -> Result<ExportSource, CliError> {
    match name {
        CommandName::Christoffel => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            Ok(ExportSource::Classes { label: "Gamma", classes: classify_grid(gamma), chart: chart.clone(), variance: CHRISTOFFEL_VARIANCE.to_vec() })
        }
        CommandName::Riemann => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            match source {
                GammaSource::FromMetric(m) => {
                    let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                    let riem_cov = lower_first_index_checkpointed(&model.registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
                    let riemann_head = declare_internal_head(&mut model.registry, m.head, 4, "__Riemann")?;
                    let tensor = grid_to_component_tensor(&model.registry, riemann_head, &riem_cov);
                    let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                    Ok(ExportSource::Classes { label: "R", classes, chart: m.chart, variance: RIEMANN_COV_VARIANCE.to_vec() })
                }
                GammaSource::FromConnection { chart, gamma } => {
                    let riem_mixed = riemann_mixed_checkpointed(&chart, &gamma, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                    Ok(ExportSource::Classes { label: "R", classes: classify_grid(&riem_mixed), chart, variance: RIEMANN_MIXED_VARIANCE.to_vec() })
                }
            }
        }
        CommandName::Ricci => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            match source {
                GammaSource::FromMetric(m) => {
                    let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                    let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                    let ricci_head = declare_internal_head(&mut model.registry, m.head, 2, "__Ricci")?;
                    let tensor = grid_to_component_tensor(&model.registry, ricci_head, &ricci);
                    let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
                    Ok(ExportSource::Classes { label: "Ricci", classes, chart: m.chart, variance: RICCI_VARIANCE.to_vec() })
                }
                GammaSource::FromConnection { chart, gamma } => {
                    let riem_mixed = riemann_mixed_checkpointed(&chart, &gamma, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
                    let ricci = ricci_tensor_checkpointed(&chart, &riem_mixed, &mut || ctx.is_cancelled())?;
                    check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
                    Ok(ExportSource::Classes { label: "Ricci", classes: classify_grid(&ricci), chart, variance: RICCI_VARIANCE.to_vec() })
                }
            }
        }
        CommandName::Scalar => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "scalar")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let r = ricci_scalar(&m.chart, &ricci, &m.ginv);
            Ok(ExportSource::Scalar(r))
        }
        CommandName::Kretschmann => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "kretschmann")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let riem_cov = lower_first_index_checkpointed(&model.registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
            let k = kretschmann_checkpointed(&m.chart, &riem_cov, &m.ginv, &mut || ctx.is_cancelled())?;
            Ok(ExportSource::Scalar(k))
        }
        CommandName::Einstein => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "einstein")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
            let einstein = einstein_tensor_checkpointed(&model.registry, &m.chart, &m.tensor, &ricci, &scalar, &mut || ctx.is_cancelled())?;
            check_grid_budget(&einstein, "einstein_tensor", args.max_nodes)?;
            let einstein_head = declare_internal_head(&mut model.registry, m.head, 2, "__Einstein")?;
            let tensor = grid_to_component_tensor(&model.registry, einstein_head, &einstein);
            let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
            Ok(ExportSource::Classes { label: "G", classes, chart: m.chart, variance: RICCI_VARIANCE.to_vec() })
        }
        CommandName::RicciSquared => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "riccisquare")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let rsq = ricci_squared_checkpointed(&m.chart, &ricci, &m.ginv, &mut || ctx.is_cancelled())?;
            Ok(ExportSource::Scalar(rsq))
        }
        CommandName::GaussBonnet => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "gaussbonnet")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let riem_cov = lower_first_index_checkpointed(&model.registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
            let k = kretschmann_checkpointed(&m.chart, &riem_cov, &m.ginv, &mut || ctx.is_cancelled())?;
            let rsq = ricci_squared_checkpointed(&m.chart, &ricci, &m.ginv, &mut || ctx.is_cancelled())?;
            Ok(ExportSource::Scalar(gauss_bonnet(&k, &rsq, &scalar)))
        }
        CommandName::Weyl => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "weyl")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let riem_cov = lower_first_index_checkpointed(&model.registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
            let weyl = weyl_tensor_checkpointed(&model.registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &mut || ctx.is_cancelled())?;
            check_grid_budget(&weyl, "weyl_tensor", args.max_nodes)?;
            let weyl_head = declare_internal_head(&mut model.registry, m.head, 4, "__Weyl")?;
            let tensor = grid_to_component_tensor(&model.registry, weyl_head, &weyl);
            let classes = classify_tensor(&model.registry, &tensor, m.chart.dim());
            Ok(ExportSource::Classes { label: "C", classes, chart: m.chart, variance: RIEMANN_COV_VARIANCE.to_vec() })
        }
        CommandName::WeylSquared => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let m = require_metric(source, "weylsquare")?;
            let riem_mixed = riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_mixed, "riemann_mixed", args.max_nodes)?;
            let riem_cov = lower_first_index_checkpointed(&model.registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())?;
            check_grid_budget(&riem_cov, "lower_first_index", args.max_nodes)?;
            let ricci = ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled())?;
            check_grid_budget(&ricci, "ricci_tensor", args.max_nodes)?;
            let scalar = ricci_scalar(&m.chart, &ricci, &m.ginv);
            let wsq =
                weyl_squared_checkpointed(&model.registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &m.ginv, &mut || ctx.is_cancelled())?;
            Ok(ExportSource::Scalar(wsq))
        }
        CommandName::Geodesic => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            let param = args.param.clone().expect("export_cmd already checked geodesic/accel have --param before calling this");
            check_affine_parameter_collision(model, chart, &source, &param, "geodesic")?;
            let equations = geodesic_equations_checkpointed(chart, gamma, &param, &mut || ctx.is_cancelled())?;
            check_grid_budget_for_equations(&equations, "geodesic", args.max_nodes)?;
            Ok(ExportSource::Equations { param, equations })
        }
        CommandName::Accel => {
            let source = resolve_gamma_source(model, args.connection.as_deref(), args.metric.as_deref(), args.max_nodes, ctx)?;
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            let param = args.param.clone().expect("export_cmd already checked geodesic/accel have --param before calling this");
            check_affine_parameter_collision(model, chart, &source, &param, "accel")?;
            let rhs = accel_equations_checkpointed(chart, gamma, &param, &mut || ctx.is_cancelled())?;
            check_grid_budget_for_equations(&rhs, "accel", args.max_nodes)?;
            Ok(ExportSource::AccelEquations { chart: chart.clone(), param, rhs })
        }
    }
}

/// Detects and renames any reserved-word collision (`oderom_expr::export`),
/// then renders `source` to `target` with NO truncation (`usize::MAX`
/// lines -- export exists to hand something complete to another tool,
/// so silently dropping components the way the on-screen `--max-lines`
/// preview does would defeat the point). See
/// `oderom-session::run::render_export`'s own doc comment for the full
/// reasoning; this is that same design, reimplemented for the plain
/// joined-string CLI output shape instead of `RenderedClasses`.
fn render_export_text(source: ExportSource, target: Target) -> String {
    let mut vars = BTreeSet::new();
    let mut funcs = BTreeSet::new();
    match &source {
        ExportSource::Classes { classes, .. } => {
            for c in classes {
                collect_names(&c.value, &mut vars, &mut funcs);
            }
        }
        ExportSource::Scalar(e) => collect_names(e, &mut vars, &mut funcs),
        ExportSource::Equations { param, equations } => {
            vars.insert(param.clone());
            for e in equations {
                collect_names(e, &mut vars, &mut funcs);
            }
        }
        ExportSource::AccelEquations { param, rhs, .. } => {
            vars.insert(param.clone());
            for e in rhs {
                collect_names(e, &mut vars, &mut funcs);
            }
        }
    }

    let (reserved, suffix): (&[&str], &str) = match target {
        Target::Mathematica => (MATHEMATICA_RESERVED, "$"),
        Target::Sympy => (SYMPY_RESERVED, "_"),
        _ => unreachable!("export_cmd only ever calls this with a symbolic export target"),
    };
    let all_names: BTreeSet<String> = vars.iter().chain(funcs.iter()).cloned().collect();
    let rename_map = build_rename_map(&all_names, reserved, suffix);
    let rename = |s: &str| rename_map.get(s).cloned().unwrap_or_else(|| s.to_string());

    let mut lines = Vec::new();
    if target == Target::Sympy {
        let renamed_vars: BTreeSet<String> = vars.iter().map(|v| rename(v)).collect();
        if !renamed_vars.is_empty() {
            let names_list = renamed_vars.iter().cloned().collect::<Vec<_>>().join(", ");
            let quoted = renamed_vars.iter().cloned().collect::<Vec<_>>().join(" ");
            lines.push(format!("{names_list} = symbols('{quoted}')"));
        }
    }

    // Every OTHER line this function adds below is a real formula --
    // valid, standalone `target` source on its own. `RenderedComponent`/
    // `RenderedClasses` can also carry plain-English annotations beside
    // a formula (an orbit-size note, a truncation/zero count) that are
    // NOT code -- mixing one of those in bare, the way the Unicode/LaTeX
    // renderers' own `to_text()` join always has, would break `exec`/
    // evaluation the instant this text is pasted into the target tool.
    // Every such line goes through `comment_line` instead, so it stays
    // present (nothing is silently dropped) but never breaks the paste.
    let push_structured = |lines: &mut Vec<String>, structured: oderom_components::RenderedClasses| {
        for c in &structured.components {
            lines.push(c.formula.clone());
            if let Some(note) = &c.orbit_note {
                lines.push(comment_line(note, target));
            }
        }
        for s in &structured.summary {
            lines.push(comment_line(s, target));
        }
    };

    match source {
        ExportSource::Classes { label, classes, chart, variance } => {
            let renamed_classes = classes
                .into_iter()
                .map(|c| ComponentClass { representative: c.representative, orbit_size: c.orbit_size, value: apply_renames(&c.value, &rename_map) })
                .collect();
            let structured = oderom_components::render_classes_structured(label, renamed_classes, &chart, &variance, target, usize::MAX);
            push_structured(&mut lines, structured);
        }
        ExportSource::Scalar(e) => lines.push(apply_renames(&e, &rename_map).render(target)),
        ExportSource::Equations { param, equations } => {
            let renamed_param = rename(&param);
            let renamed: Vec<Expr> = equations.iter().map(|e| apply_renames(e, &rename_map)).collect();
            let structured = oderom_components::render_geodesic_structured("geodesic", &renamed, &renamed_param, target, usize::MAX);
            push_structured(&mut lines, structured);
        }
        ExportSource::AccelEquations { chart, param, rhs } => {
            let renamed_param = rename(&param);
            let renamed: Vec<Expr> = rhs.iter().map(|e| apply_renames(e, &rename_map)).collect();
            let structured = oderom_components::render_geodesic_solved_structured("accel", &chart, &renamed, &renamed_param, target, usize::MAX);
            push_structured(&mut lines, structured);
        }
    }

    if !rename_map.is_empty() {
        let target_word = if target == Target::Mathematica { "Mathematica" } else { "SymPy" };
        for (orig, renamed) in &rename_map {
            lines.push(comment_line(&format!("renamed `{orig}` to `{renamed}` -- collides with a reserved {target_word} name"), target));
        }
    }

    lines.join("\n")
}
