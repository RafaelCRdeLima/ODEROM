//! Runs one parsed `Query` against a `Document`, cache-aware --
//! DESIGN-UI-SESSION.md sections 2/3. Reuses `oderom-cli::commands`'
//! name-resolution (`resolve_gamma_source`) and rendering
//! (`classify_tensor`/`render_classes`) exactly as the CLI's own five
//! subcommands do, rather than a second implementation of either: the
//! only thing genuinely new here is wrapping the expensive stages in
//! the compute cache, and rendering to both targets from one cached
//! value instead of recomputing per target.
//!
//! Cancellable throughout: every stage call is the `_checkpointed`
//! variant, polling `ctx.is_cancelled()` once per independent component
//! (`oderom-repl`'s Ctrl+C handler calls `ctx.cancel()` from the main
//! thread while this runs on a worker thread -- ETAPA 2). `ctx` is
//! always caller-provided, never constructed here, so the caller (the
//! REPL, or a test) can share one `ExecutionContext` across the thread
//! boundary.

use crate::cache::ComputeCache;
use crate::document::Document;
use crate::entry::EntryResult;
use crate::fingerprint::composite_fingerprint;
use oderom_cli::commands::{
    check_affine_parameter_collision, check_grid_budget, declare_internal_head, require_metric, resolve_gamma_source,
    ExecutionContext, GammaSource,
};
use oderom_cli::parser::{CommandName, Query};
use oderom_cli::span::Spanned;
use oderom_cli::CliError;
use oderom_expr::export::{apply_renames, build_rename_map, collect_names, MATHEMATICA_RESERVED, SYMPY_RESERVED};
use oderom_components::curvature::{
    accel_equations_checkpointed, change_variance_checkpointed, einstein_tensor_checkpointed, gauss_bonnet,
    geodesic_equations_checkpointed, grid_to_component_tensor, kretschmann_checkpointed, lower_first_index_checkpointed, ricci_scalar,
    ricci_squared, ricci_tensor_checkpointed, riemann_mixed_checkpointed, weyl_squared_checkpointed, weyl_tensor_checkpointed,
};
use oderom_components::{
    classify_grid, classify_tensor, render_classes, render_classes_structured, render_geodesic, render_geodesic_solved,
    render_geodesic_solved_structured, render_geodesic_structured, Chart, RenderedClasses, RenderedComponent, CHRISTOFFEL_VARIANCE,
    RICCI_VARIANCE, RIEMANN_COV_VARIANCE, RIEMANN_MIXED_VARIANCE,
};
use oderom_core::{Render, Target, Variance};
use std::collections::{BTreeMap, BTreeSet};

/// Same default as the CLI's own (`commands.rs`'s `DEFAULT_MAX_NODES`,
/// kept private there) -- not shared as a `pub const` yet because
/// nothing has needed the two to be configurable independently; revisit
/// if that changes rather than guessing now.
const MAX_NODES: usize = 20_000;
const MAX_LINES: usize = 20;

/// Runs `query` against `document`, using and populating `cache` for
/// the expensive stages, checking `ctx.is_cancelled()` throughout.
/// Returns the rendered result (both targets -- see module docs) and
/// the set of declared names the query depended on (for the caller to
/// store on the `Entry`, DESIGN-UI-SESSION.md section 2).
///
/// Wrapped in `run_cancellable` so `oderom_expr`'s deep, per-loop checks
/// (inside `normalize()`/`poly_gcd`/the subresultant PRS) are armed for
/// the query's whole duration, not just the between-component checks
/// the `_checkpointed` stage functions already do -- a real gap found
/// against a metric whose `g_tt`/`g_rr` are not reciprocal, where a
/// *single* component's `normalize()` call ran away and the
/// between-component checkpoint was never reached because that one call
/// never returned to it.
pub fn run_query(
    document: &Document,
    cache: &mut ComputeCache,
    query: &Query,
    ctx: &ExecutionContext,
) -> Result<(EntryResult, BTreeSet<String>), CliError> {
    match oderom_expr::run_cancellable(ctx.token(), || run_query_inner(document, cache, query, ctx)) {
        Ok(result) => result,
        // Same variant (and so the same rendered "cancelled" message)
        // the between-component checkpoint already produces via `?` --
        // one consistent, already-tested error shape for either path
        // that reaches it, not two.
        Err(_cancelled) => Err(oderom_components::ComponentError::Cancelled.into()),
    }
}

fn run_query_inner(
    document: &Document,
    cache: &mut ComputeCache,
    query: &Query,
    ctx: &ExecutionContext,
) -> Result<(EntryResult, BTreeSet<String>), CliError> {
    let start = oderom_core::clock::Instant::now();

    // `export`'s own one-shot text result (Rodada Exportação) bypasses
    // the Latex+Unicode dual-render below entirely -- see
    // `render_export`'s own doc comment for why a third, dedicated path
    // rather than a third pair of fields on `EntryResult`. The INNER
    // query still runs through the exact same `run_query_to_rendered`
    // every other query uses, so it inherits that call's caching,
    // staleness tracking (`used`), and -- because this whole function is
    // always invoked from inside `run_query`'s `run_cancellable` wrapper
    // -- its cancellation, unchanged; only what happens to the already-
    // computed `Rendered` value afterward differs.
    if let Query::Export { format, inner } = query {
        let (rendered, used) = run_query_to_rendered(document, cache, inner, ctx)?;
        let structured = render_export(&rendered, format.value);
        let elapsed_ms = start.elapsed().as_millis() as u64;
        // Flat text (what a REPL would print, `EntryResult.unicode`/
        // `.latex`) wraps every plain-English annotation as a real
        // comment first -- see `export_flat_text`'s own doc comment for
        // why this must NOT be `structured.to_text()` unchanged, and why
        // `structured.components`/`.summary` themselves stay untouched
        // below (the notebook's own structured display needs them
        // comment-marker-free).
        let text = export_flat_text(&structured, format.value);
        return Ok((
            EntryResult { latex: text.clone(), unicode: text, components: structured.components, summary: structured.summary, elapsed_ms },
            used,
        ));
    }

    let (rendered, used) = run_query_to_rendered(document, cache, query, ctx)?;

    let latex = rendered.render(Target::Latex, MAX_LINES);
    let unicode = rendered.render(Target::Unicode, MAX_LINES);
    // Structured, LaTeX-only form for the notebook/app frontend
    // (`oderom-app::entry_state_dto`), which never shows `unicode` at
    // all -- see that crate's own `lib.rs` doc comment. `latex`/
    // `unicode` above stay the flat, single-string shape
    // (`oderom-repl` still `println!`s `.unicode` directly, and the
    // CLI's own `render_classes` is the same function under the hood),
    // untouched by this addition; `components`/`summary` are additive,
    // never a replacement for them.
    let structured = rendered.render_structured(Target::Latex, MAX_LINES);
    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok((EntryResult { latex, unicode, components: structured.components, summary: structured.summary, elapsed_ms }, used))
}

/// The part of `run_query_inner` genuinely shared with `export`: runs
/// `query` all the way down to one cached, already-computed [`Rendered`]
/// value, with no target-specific rendering applied yet. Extracted out
/// so `Query::Export`'s own handling above can run exactly this same
/// path over its `inner` query, rather than a second, parallel
/// implementation of `Query::Command`/`Geodesic`/`Accel` dispatch that
/// could drift from this one.
fn run_query_to_rendered(
    document: &Document,
    cache: &mut ComputeCache,
    query: &Query,
    ctx: &ExecutionContext,
) -> Result<(Rendered, BTreeSet<String>), CliError> {
    match query {
        Query::Command { name, target, variance } => {
            run_command_query(document, cache, name.value, target.as_ref().map(|t| t.value.as_str()), variance.as_ref(), ctx)
        }
        Query::Geodesic { target, param } => {
            let target_name = target.as_ref().map(|t| t.value.as_str());
            let source = resolve_gamma_source(&document.model, None, target_name, MAX_NODES, ctx)?;
            let used = ctx.used();
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            check_affine_parameter_collision(&document.model, chart, &source, &param.value, "geodesic")?;
            let equations = geodesic_equations_checkpointed(chart, gamma, &param.value, &mut || ctx.is_cancelled())?;
            Ok((Rendered::Equations { param: param.value.clone(), equations }, used))
        }
        Query::Accel { target, param } => {
            let target_name = target.as_ref().map(|t| t.value.as_str());
            let source = resolve_gamma_source(&document.model, None, target_name, MAX_NODES, ctx)?;
            let used = ctx.used();
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            check_affine_parameter_collision(&document.model, chart, &source, &param.value, "accel")?;
            let rhs = accel_equations_checkpointed(chart, gamma, &param.value, &mut || ctx.is_cancelled())?;
            Ok((Rendered::AccelEquations { chart: chart.clone(), param: param.value.clone(), rhs }, used))
        }
        // The parser never builds an `Export` whose own `inner` is
        // itself an `Export` (`oderom-cli::parser::parse_export_query_tail`
        // rejects that at parse time), and `run_query_inner` above
        // already intercepts a TOP-LEVEL `Query::Export` before this
        // function is ever called with one -- so this arm can never
        // actually run.
        Query::Export { .. } => unreachable!("export is never nested and is intercepted before reaching run_query_to_rendered"),
    }
}

/// The existing ten commands' body, unchanged in behavior when no
/// index-variance marker is present -- extracted out of `run_query_inner`
/// only so the new `Query::Geodesic` arm (which shares nothing with this
/// beyond the final render/`EntryResult` step) doesn't have to sit
/// inside the same `match` as this one's arms.
///
/// `variance` (Rodada Variancia: `riemann [up,down,down,down]`) is
/// handled entirely by its own early-returning branch below, BEFORE the
/// unmarked path even runs -- that unmarked path (the big `match name`
/// a few lines down) is untouched by this addition, byte for byte, so
/// every query without a marker keeps producing exactly what it always
/// has (the round's own non-regression requirement).
fn run_command_query(
    document: &Document,
    cache: &mut ComputeCache,
    name: CommandName,
    target_name: Option<&str>,
    variance: Option<&Spanned<Vec<Variance>>>,
    ctx: &ExecutionContext,
) -> Result<(Rendered, BTreeSet<String>), CliError> {
    if let Some(v) = variance {
        // Which commands even accept a marker (barred for Christoffel --
        // not a tensor; meaningless for the five bare scalars -- no
        // indices) and the expected rank for the four that do: checked
        // BEFORE `resolve_gamma_source`, so a doomed query fails fast
        // without loading/inverting a metric it was never going to use.
        let command_word: &str = match name {
            CommandName::Christoffel => return Err(CliError::ChristoffelNotATensor),
            CommandName::Scalar => return Err(CliError::VarianceOnScalar { name: "scalar".to_string() }),
            CommandName::Kretschmann => return Err(CliError::VarianceOnScalar { name: "kretschmann".to_string() }),
            CommandName::RicciSquared => return Err(CliError::VarianceOnScalar { name: "riccisquare".to_string() }),
            CommandName::GaussBonnet => return Err(CliError::VarianceOnScalar { name: "gaussbonnet".to_string() }),
            CommandName::WeylSquared => return Err(CliError::VarianceOnScalar { name: "weylsquare".to_string() }),
            CommandName::Riemann => "riemann",
            CommandName::Ricci => "ricci",
            CommandName::Einstein => "einstein",
            CommandName::Weyl => "weyl",
            CommandName::Geodesic | CommandName::Accel => unreachable!("geodesic/accel never reach run_command_query, see the module doc comment"),
        };
        let expected_rank = match name {
            CommandName::Riemann | CommandName::Weyl => 4,
            CommandName::Ricci | CommandName::Einstein => 2,
            _ => unreachable!("Christoffel and the five scalars already returned above"),
        };
        if v.value.len() != expected_rank {
            return Err(CliError::VarianceArityMismatch { command: command_word.to_string(), expected: expected_rank, found: v.value.len() });
        }

        // A variance marker always needs a real metric -- even for
        // Riemann/Ricci, which normally work from a bare connection too
        // -- the same refusal `scalar`/`kretschmann` already give a bare
        // connection, reused verbatim via `require_metric`.
        let source = resolve_gamma_source(&document.model, None, target_name, MAX_NODES, ctx)?;
        let used = ctx.used();
        let fp = composite_fingerprint(&document.fingerprints, &used);
        let registry = document.model.registry.clone();
        let m = require_metric(source, command_word)?;

        let riem_mixed = cache
            .riemann_mixed
            .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
            .clone();
        check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;

        // The tensor's own natural, fully covariant form -- exactly what
        // the unmarked query below would show, before any variance
        // change -- computed via the same cached stages that path uses.
        let (natural_cov, label) = match name {
            CommandName::Riemann => {
                let riem_cov = cache
                    .riemann_cov
                    .get_or_try_insert_with(fp, || {
                        lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                    })?
                    .clone();
                check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
                (riem_cov, "R")
            }
            CommandName::Ricci => {
                let ricci = cache
                    .ricci_tensor
                    .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
                (ricci, "Ricci")
            }
            CommandName::Einstein => {
                let ricci = cache
                    .ricci_tensor
                    .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
                let scalar =
                    cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
                let einstein = einstein_tensor_checkpointed(&registry, &m.chart, &m.tensor, &ricci, &scalar, &mut || ctx.is_cancelled())?;
                check_grid_budget(&einstein, "einstein_tensor", MAX_NODES)?;
                (einstein, "G")
            }
            CommandName::Weyl => {
                let riem_cov = cache
                    .riemann_cov
                    .get_or_try_insert_with(fp, || {
                        lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                    })?
                    .clone();
                check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
                let ricci = cache
                    .ricci_tensor
                    .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
                let scalar =
                    cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
                let weyl = cache
                    .weyl_tensor
                    .get_or_try_insert_with(fp, || {
                        weyl_tensor_checkpointed(&registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &mut || ctx.is_cancelled())
                    })?
                    .clone();
                check_grid_budget(&weyl, "weyl_tensor", MAX_NODES)?;
                (weyl, "C")
            }
            _ => unreachable!("Christoffel and the five scalars already returned above"),
        };

        let current = vec![Variance::Co; expected_rank];
        let changed =
            change_variance_checkpointed(&registry, &m.chart, &natural_cov, &current, &v.value, &m.tensor, &m.ginv, &mut || ctx.is_cancelled())?;
        let rendered = Rendered::Classes { label: label.to_string(), classes: classify_grid(&changed), chart: m.chart.clone(), variance: v.value.clone() };
        return Ok((rendered, used));
    }

    let source = resolve_gamma_source(&document.model, None, target_name, MAX_NODES, ctx)?;
    let used = ctx.used();
    let fp = composite_fingerprint(&document.fingerprints, &used);

    let mut registry = document.model.registry.clone();

    let rendered = match name {
        // The parser never builds a `Query::Command` with this name --
        // `parse_query_tokens` routes `geodesic` through `Query::Geodesic`
        // instead, before `CommandName::Geodesic` could ever reach here
        // (`oderom-cli/src/parser.rs`'s own `if name == CommandName::Geodesic`
        // branch, checked before the single-optional-target parsing this
        // function's caller uses).
        CommandName::Geodesic => unreachable!("`geodesic` is always parsed into Query::Geodesic, never Query::Command"),
        CommandName::Accel => unreachable!("`accel` is always parsed into Query::Accel, never Query::Command"),
        CommandName::Christoffel => {
            let (chart, gamma) = match &source {
                GammaSource::FromMetric(m) => (&m.chart, &m.gamma),
                GammaSource::FromConnection { chart, gamma } => (chart, gamma),
            };
            Rendered::Classes {
                label: "Gamma".to_string(),
                classes: classify_grid(gamma),
                chart: chart.clone(),
                variance: CHRISTOFFEL_VARIANCE.to_vec(),
            }
        }
        CommandName::Riemann => match source {
            GammaSource::FromMetric(m) => {
                let riem_mixed = cache
                    .riemann_mixed
                    .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
                let riem_cov = cache
                    .riemann_cov
                    .get_or_try_insert_with(fp, || {
                        lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                    })?
                    .clone();
                check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
                let riemann_head = declare_internal_head(&mut registry, m.head, 4, "__Riemann")?;
                let tensor = grid_to_component_tensor(&registry, riemann_head, &riem_cov);
                let classes = classify_tensor(&registry, &tensor, m.chart.dim());
                Rendered::Classes { label: "R".to_string(), classes, chart: m.chart.clone(), variance: RIEMANN_COV_VARIANCE.to_vec() }
            }
            GammaSource::FromConnection { chart, gamma } => {
                let riem_mixed = cache
                    .riemann_mixed
                    .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&chart, &gamma, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
                Rendered::Classes {
                    label: "R".to_string(),
                    classes: classify_grid(&riem_mixed),
                    chart: chart.clone(),
                    variance: RIEMANN_MIXED_VARIANCE.to_vec(),
                }
            }
        },
        CommandName::Ricci => match source {
            GammaSource::FromMetric(m) => {
                let riem_mixed = cache
                    .riemann_mixed
                    .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
                let ricci = cache
                    .ricci_tensor
                    .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
                let ricci_head = declare_internal_head(&mut registry, m.head, 2, "__Ricci")?;
                let tensor = grid_to_component_tensor(&registry, ricci_head, &ricci);
                let classes = classify_tensor(&registry, &tensor, m.chart.dim());
                Rendered::Classes { label: "Ricci".to_string(), classes, chart: m.chart.clone(), variance: RICCI_VARIANCE.to_vec() }
            }
            GammaSource::FromConnection { chart, gamma } => {
                let riem_mixed = cache
                    .riemann_mixed
                    .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&chart, &gamma, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
                let ricci = cache
                    .ricci_tensor
                    .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                    .clone();
                check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
                Rendered::Classes {
                    label: "Ricci".to_string(),
                    classes: classify_grid(&ricci),
                    chart: chart.clone(),
                    variance: RICCI_VARIANCE.to_vec(),
                }
            }
        },
        CommandName::Scalar => {
            let m = require_metric(source, "scalar")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            // ricci_scalar has no _checkpointed variant (rank-2 -> rank-0
            // contraction, n^2 terms -- cheap, same reasoning as
            // ricci_tensor's own doc comment on why it still has one for
            // consistency but this one doesn't bother).
            let r = cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
            Rendered::Scalar(r)
        }
        CommandName::Einstein => {
            let m = require_metric(source, "einstein")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            let scalar =
                cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
            // Not its own cache slot: the combination itself is cheap
            // (one subtraction per component, `einstein_tensor`'s own
            // doc comment) -- both real, expensive inputs (`ricci`,
            // `scalar`) are already memoized above, the same "don't
            // cache what's cheap to redo" precedent `geodesic`/`accel`
            // already set.
            let einstein = einstein_tensor_checkpointed(&registry, &m.chart, &m.tensor, &ricci, &scalar, &mut || ctx.is_cancelled())?;
            check_grid_budget(&einstein, "einstein_tensor", MAX_NODES)?;
            let einstein_head = declare_internal_head(&mut registry, m.head, 2, "__Einstein")?;
            let tensor = grid_to_component_tensor(&registry, einstein_head, &einstein);
            let classes = classify_tensor(&registry, &tensor, m.chart.dim());
            Rendered::Classes { label: "G".to_string(), classes, chart: m.chart.clone(), variance: RICCI_VARIANCE.to_vec() }
        }
        CommandName::RicciSquared => {
            let m = require_metric(source, "riccisquare")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            // Not its own cache slot -- same "cheap to redo from an
            // already-cached input" reasoning as `ricci_scalar`'s own
            // doc comment (an `n^2` contraction, not a stage on the
            // order of `riemann_mixed`/`riemann_cov`).
            let rsq = ricci_squared(&m.chart, &ricci, &m.ginv);
            Rendered::Scalar(rsq)
        }
        CommandName::GaussBonnet => {
            let m = require_metric(source, "gaussbonnet")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let riem_cov = cache
                .riemann_cov
                .get_or_try_insert_with(fp, || {
                    lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                })?
                .clone();
            check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            let scalar =
                cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
            let k = cache
                .kretschmann
                .get_or_try_insert_with(fp, || kretschmann_checkpointed(&m.chart, &riem_cov, &m.ginv, &mut || ctx.is_cancelled()))?
                .clone();
            let rsq = ricci_squared(&m.chart, &ricci, &m.ginv);
            // Not its own cache slot: an O(1) recombination of three
            // already-cached scalars, same reasoning as
            // `einstein_tensor`'s own uncached final subtraction.
            Rendered::Scalar(gauss_bonnet(&k, &rsq, &scalar))
        }
        CommandName::Kretschmann => {
            let m = require_metric(source, "kretschmann")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let riem_cov = cache
                .riemann_cov
                .get_or_try_insert_with(fp, || {
                    lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                })?
                .clone();
            check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
            let k = cache
                .kretschmann
                .get_or_try_insert_with(fp, || kretschmann_checkpointed(&m.chart, &riem_cov, &m.ginv, &mut || ctx.is_cancelled()))?
                .clone();
            Rendered::Scalar(k)
        }
        CommandName::Weyl => {
            let m = require_metric(source, "weyl")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let riem_cov = cache
                .riemann_cov
                .get_or_try_insert_with(fp, || {
                    lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                })?
                .clone();
            check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            let scalar =
                cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
            let weyl = cache
                .weyl_tensor
                .get_or_try_insert_with(fp, || {
                    weyl_tensor_checkpointed(&registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &mut || ctx.is_cancelled())
                })?
                .clone();
            check_grid_budget(&weyl, "weyl_tensor", MAX_NODES)?;
            let weyl_head = declare_internal_head(&mut registry, m.head, 4, "__Weyl")?;
            let tensor = grid_to_component_tensor(&registry, weyl_head, &weyl);
            let classes = classify_tensor(&registry, &tensor, m.chart.dim());
            Rendered::Classes { label: "C".to_string(), classes, chart: m.chart.clone(), variance: RIEMANN_COV_VARIANCE.to_vec() }
        }
        CommandName::WeylSquared => {
            let m = require_metric(source, "weylsquare")?;
            let riem_mixed = cache
                .riemann_mixed
                .get_or_try_insert_with(fp, || riemann_mixed_checkpointed(&m.chart, &m.gamma, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&riem_mixed, "riemann_mixed", MAX_NODES)?;
            let riem_cov = cache
                .riemann_cov
                .get_or_try_insert_with(fp, || {
                    lower_first_index_checkpointed(&registry, &m.chart, &riem_mixed, &m.tensor, &mut || ctx.is_cancelled())
                })?
                .clone();
            check_grid_budget(&riem_cov, "lower_first_index", MAX_NODES)?;
            let ricci = cache
                .ricci_tensor
                .get_or_try_insert_with(fp, || ricci_tensor_checkpointed(&m.chart, &riem_mixed, &mut || ctx.is_cancelled()))?
                .clone();
            check_grid_budget(&ricci, "ricci_tensor", MAX_NODES)?;
            let scalar =
                cache.ricci_scalar.get_or_try_insert_with(fp, || Ok::<_, CliError>(ricci_scalar(&m.chart, &ricci, &m.ginv)))?.clone();
            let wsq = cache
                .weyl_squared
                .get_or_try_insert_with(fp, || {
                    weyl_squared_checkpointed(&registry, &m.chart, &m.tensor, &riem_cov, &ricci, &scalar, &m.ginv, &mut || {
                        ctx.is_cancelled()
                    })
                })?
                .clone();
            Rendered::Scalar(wsq)
        }
    };

    Ok((rendered, used))
}

/// A value late enough in the pipeline to render to either `Target`
/// without recomputing anything -- exactly the "cache the semantic
/// object, render on demand" split DESIGN-UI-SESSION.md section 3
/// (verification 2, point 2) asks for: switching target or `max_lines`
/// only ever re-enters this, never the cache above. `chart`/`variance`
/// travel with `Classes` (not just `label`/`classes`) because naming
/// indices and placing them by variance (Problem 1, this round) needs
/// both, and neither is recoverable from `classes` alone.
enum Rendered {
    Classes { label: String, classes: Vec<oderom_components::ComponentClass>, chart: Chart, variance: Vec<Variance> },
    Scalar(oderom_expr::Expr),
    /// `geodesic`'s own output (Marco 6 step 4, round B) -- one equation
    /// per coordinate, already normalized. No `chart` needed here (unlike
    /// `Classes` above): each equation already names its own coordinates
    /// via the `Expr::Func` nodes `geodesic_equations` built, nothing
    /// external to place them by. `param` is what tells the renderer
    /// which `Expr::Func` nodes are "coordinate as a function of the
    /// affine parameter" (dot notation in LaTeX) rather than an ordinary
    /// indeterminate function -- see `oderom_components::render_geodesic`'s
    /// own doc comment for the full design point.
    Equations { param: String, equations: Vec<oderom_expr::Expr> },
    /// `accel`'s own output (Marco 6 step 4, round C) -- one
    /// already-solved-and-normalized right-hand side per coordinate, in
    /// chart order. Unlike `Equations` above, `chart` IS needed here:
    /// each solved right-hand side no longer contains its own
    /// coordinate's acceleration node (that's the whole point of
    /// solving for it), so there is nothing left in the expression
    /// itself to name the left-hand side by -- `render_geodesic_solved`
    /// reads it positionally off `chart.coords` instead.
    AccelEquations { chart: Chart, param: String, rhs: Vec<oderom_expr::Expr> },
}

impl Rendered {
    fn render(&self, target: Target, max_lines: usize) -> String {
        match self {
            Rendered::Classes { label, classes, chart, variance } => render_classes(label, classes.clone(), chart, variance, target, max_lines),
            Rendered::Scalar(e) => e.render(target),
            Rendered::Equations { param, equations } => render_geodesic("geodesic", equations, param, target, max_lines),
            Rendered::AccelEquations { chart, param, rhs } => render_geodesic_solved("accel", chart, rhs, param, target, max_lines),
        }
    }

    /// Same value, split into clickable/KaTeX-able pieces rather than
    /// one joined string -- see [`oderom_components::RenderedClasses`].
    /// A bare scalar (`scalar`/`kretschmann`) has no orbit note and no
    /// summary lines to speak of; it still comes back as one
    /// `RenderedComponent` so the frontend can treat "click a result to
    /// copy its LaTeX" uniformly rather than special-casing scalars.
    fn render_structured(&self, target: Target, max_lines: usize) -> RenderedClasses {
        match self {
            Rendered::Classes { label, classes, chart, variance } => {
                render_classes_structured(label, classes.clone(), chart, variance, target, max_lines)
            }
            Rendered::Scalar(e) => RenderedClasses {
                label: String::new(),
                components: vec![RenderedComponent { formula: e.render(target), orbit_note: None }],
                summary: Vec::new(),
            },
            Rendered::Equations { param, equations } => render_geodesic_structured("geodesic", equations, param, target, max_lines),
            Rendered::AccelEquations { chart, param, rhs } => render_geodesic_solved_structured("accel", chart, rhs, param, target, max_lines),
        }
    }

    /// Every plain variable and indeterminate-function name this result
    /// actually uses, for `render_export`'s own reserved-word check
    /// (Rodada Exportação) -- kept separate, not folded into `render`/
    /// `render_structured` above, because it needs to walk the RAW
    /// `Expr` trees (`oderom_expr::export::collect_names`), not produce
    /// rendered text.
    ///
    /// Deliberately does NOT walk `chart.coords` or the tensor `label`:
    /// both are always rendered fused into one identifier alongside an
    /// index suffix (`R` + `_t_r_t_r` -> `R_t_r_t_r`, see
    /// `oderom_components::format_indices`'s own doc comment) or quoted
    /// as a Python string literal (`Function('t')`), never emitted as a
    /// bare, standalone identifier -- so neither can actually collide
    /// with a target's reserved word, regardless of what it's spelled.
    /// `param` (the affine parameter) is the one exception collected
    /// explicitly here rather than left to turn up inside `equations`/
    /// `rhs`: it almost always does (every velocity term references it),
    /// but nothing guarantees it for every possible equation set, and
    /// unlike the label/chart case above, `param` DOES appear bare (as
    /// `Derivative(..., tau, ...)`'s `tau`) in the exported text.
    fn collect_export_names(&self, vars: &mut BTreeSet<String>, funcs: &mut BTreeSet<String>) {
        match self {
            Rendered::Classes { classes, .. } => {
                for c in classes {
                    collect_names(&c.value, vars, funcs);
                }
            }
            Rendered::Scalar(e) => collect_names(e, vars, funcs),
            Rendered::Equations { param, equations } => {
                vars.insert(param.clone());
                for e in equations {
                    collect_names(e, vars, funcs);
                }
            }
            Rendered::AccelEquations { param, rhs, .. } => {
                vars.insert(param.clone());
                for e in rhs {
                    collect_names(e, vars, funcs);
                }
            }
        }
    }

    /// `self` with every `Expr::Var`/`Expr::Func::name` (and `param`,
    /// for the two equation variants) renamed per `map` -- pure
    /// substitution, see `oderom_expr::export::apply_renames`'s own doc
    /// comment. `chart`/`label` pass through untouched, for the same
    /// reason `collect_export_names` above never collects them.
    fn renamed(&self, map: &BTreeMap<String, String>) -> Rendered {
        let rename = |s: &str| map.get(s).cloned().unwrap_or_else(|| s.to_string());
        match self {
            Rendered::Classes { label, classes, chart, variance } => Rendered::Classes {
                label: label.clone(),
                classes: classes
                    .iter()
                    .map(|c| oderom_components::ComponentClass {
                        representative: c.representative.clone(),
                        orbit_size: c.orbit_size,
                        value: apply_renames(&c.value, map),
                    })
                    .collect(),
                chart: chart.clone(),
                variance: variance.clone(),
            },
            Rendered::Scalar(e) => Rendered::Scalar(apply_renames(e, map)),
            Rendered::Equations { param, equations } => {
                Rendered::Equations { param: rename(param), equations: equations.iter().map(|e| apply_renames(e, map)).collect() }
            }
            Rendered::AccelEquations { chart, param, rhs } => {
                Rendered::AccelEquations { chart: chart.clone(), param: rename(param), rhs: rhs.iter().map(|e| apply_renames(e, map)).collect() }
            }
        }
    }
}

/// `export`'s own render pass (Rodada Exportação): detects and renames
/// any reserved-word collision (`oderom_expr::export`), then re-renders
/// `rendered` to `target` with NO truncation (`usize::MAX` lines --
/// export exists to hand something complete to another tool, so
/// silently dropping components the way the on-screen `max_lines`
/// preview does would defeat the point). No new computation happens
/// here: `rendered` is already fully computed and normalized by
/// `run_query_to_rendered`; this only walks the existing tree once to
/// rename, then again through the ordinary `Target::Mathematica`/
/// `Target::Sympy` renderer already exercised by every other query --
/// both are cheap, linear tree walks, the same complexity class the
/// unexported `render`/`render_structured` calls above already pay with
/// no cancellation checkpoint of their own, so this adds no new
/// long-running, uncancellable path either.
///
/// For `Target::Sympy` specifically, prepends a `symbols(...)`
/// declaration line as the export's own first "component" (the user's
/// own stated preference: self-sufficient output that runs in a fresh
/// SymPy session with no separate setup step) -- built from `vars`
/// alone, never `funcs`, since an indeterminate function's name is
/// always constructed inline via `Function('f')(...)` and must never be
/// pre-bound to a Python variable (see `oderom_expr::render::sympy_func`'s
/// own doc comment for why: the same name can appear as both a bare
/// `Symbol` and a `Function(...)` of a different variable within one
/// equation set, and pre-binding it would collide the two).
fn render_export(rendered: &Rendered, target: Target) -> RenderedClasses {
    let mut vars = BTreeSet::new();
    let mut funcs = BTreeSet::new();
    rendered.collect_export_names(&mut vars, &mut funcs);

    let (reserved, suffix): (&[&str], &str) = match target {
        Target::Mathematica => (MATHEMATICA_RESERVED, "$"),
        Target::Sympy => (SYMPY_RESERVED, "_"),
        _ => unreachable!("render_export is only ever called with Target::Mathematica or Target::Sympy"),
    };
    let all_names: BTreeSet<String> = vars.iter().chain(funcs.iter()).cloned().collect();
    let rename_map = build_rename_map(&all_names, reserved, suffix);

    let mut structured = rendered.renamed(&rename_map).render_structured(target, usize::MAX);

    if target == Target::Sympy {
        let renamed_vars: BTreeSet<String> = vars.iter().map(|v| rename_map.get(v).cloned().unwrap_or_else(|| v.clone())).collect();
        if !renamed_vars.is_empty() {
            let names_list = renamed_vars.iter().cloned().collect::<Vec<_>>().join(", ");
            let quoted = renamed_vars.iter().cloned().collect::<Vec<_>>().join(" ");
            structured.components.insert(0, RenderedComponent { formula: format!("{names_list} = symbols('{quoted}')"), orbit_note: None });
        }
    }

    if !rename_map.is_empty() {
        let target_word = if target == Target::Mathematica { "Mathematica" } else { "SymPy" };
        for (orig, renamed) in rename_map.iter().rev() {
            structured.summary.insert(0, format!("note: renamed `{orig}` to `{renamed}` -- collides with a reserved {target_word} name"));
        }
    }

    structured
}

/// `structured`, flattened to one string the way `RenderedClasses::to_text()`
/// already does -- except every plain-English annotation (`orbit_note`,
/// a summary line) is wrapped as `target`'s own comment syntax first
/// (`oderom_expr::export::comment_line`). `RenderedClasses::to_text()`
/// itself can't do this: it has no `target` parameter and is shared by
/// every render target, most of which (Unicode, LaTeX) are read by a
/// person, never re-executed, so a bare prose line mixed into that text
/// is harmless there. For Mathematica/Sympy specifically, that same
/// bare line breaks `exec`/evaluation the instant this flat text is
/// pasted -- a real bug caught by actually running an export's output
/// in real SymPy (a component with a nontrivial symmetry orbit) rather
/// than only inspecting it. `structured.components`/`.summary`
/// themselves are never mutated by this function -- the notebook's own
/// structured display shows `orbit_note`/summary lines as separate,
/// comment-marker-free plain text beside the formula (`oderom-app`'s
/// `renderComponents`), which is already correct and must stay that
/// way; only the flattened join needs the comment marker.
fn export_flat_text(structured: &RenderedClasses, target: Target) -> String {
    let mut lines: Vec<String> = Vec::new();
    for c in &structured.components {
        lines.push(c.formula.clone());
        if let Some(note) = &c.orbit_note {
            lines.push(oderom_expr::export::comment_line(note, target));
        }
    }
    for s in &structured.summary {
        lines.push(oderom_expr::export::comment_line(s, target));
    }
    if lines.is_empty() {
        lines.push(format!("{}: no independent components (identically zero)", structured.label));
    }
    lines.join("\n")
}
