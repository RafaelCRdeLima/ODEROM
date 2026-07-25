//! Exercises the REAL UI layer -- the actual compiled `oderom-app`
//! window, real CodeMirror instances, real focus, real keydown events
//! -- not the Rust command handlers directly. Calling
//! `execute_block`/`edit_block` (or `cm.focus()`) straight from a test
//! would prove nothing about the keymap or focus wiring; that layer is
//! exactly what broke in an earlier round of this project's own session
//! log, diagnosed and fixed only by testing it for real
//! (`dist/keytest.js`, loaded instead of `index.html` when
//! `ODEROM_APP_TEST` is set -- see `src/lib.rs`'s
//! `frontend_entry_point`).
//!
//! This process spawns the compiled binary, waits for `keytest.js` to
//! report back via a file (the same channel `frontend_ready`/
//! `ui_test_report` already use elsewhere in this project -- there is
//! no devtools console to read here), and asserts on what actually
//! happened inside the real window.
//!
//! `HOME` is pointed at a fresh temp dir per run (same reasoning as
//! `oderom-repl/tests/end_to_end.rs`'s own `$HOME` isolation): webkit2gtk
//! keeps a persistent on-disk cache under `$HOME/.local/share/<app id>`
//! that survives across process restarts and, during this feature's own
//! development, was found to keep serving a stale `dist/*.js` across
//! runs that changed it -- an isolated `$HOME` per test run means a
//! stale cache from a previous run (or a real interactive session) can
//! never leak into what this test observes.
//!
//! Requires a real display (`DISPLAY`/Wayland) to run, same as any
//! other Tauri window -- not runnable in a purely headless CI runner
//! without a virtual framebuffer (Xvfb or equivalent). Skips itself
//! with a clear message rather than failing if no display is set, so
//! `cargo test --workspace` in an environment without one still passes
//! (the rest of the workspace does not need a display).

use serde::Deserialize;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[derive(Deserialize, Debug)]
struct Phase {
    phase: String,
    ok: bool,
    #[serde(default)]
    details: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct Report {
    ok: bool,
    phases: Vec<Phase>,
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oderom-app-keymap-test-{name}-{}-{}", std::process::id(), unique()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
    t.wrapping_add(n)
}

#[test]
fn shift_ctrl_alt_enter_and_autofocus_work_through_the_real_ui() {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipping: no DISPLAY/WAYLAND_DISPLAY -- this test drives a real window, see this file's own doc comment");
        return;
    }

    let home = tempdir("home");
    let report_path = tempdir("report").join("report.json");

    let child = Command::new(env!("CARGO_BIN_EXE_oderom-app"))
        .env("HOME", &home)
        .env("ODEROM_APP_TEST", "1")
        .env("ODEROM_APP_TEST_REPORT_PATH", &report_path)
        .spawn()
        .expect("failed to spawn oderom-app");
    let _guard = ChildGuard(child);

    // Etapa 3b, segunda parte (DESIGN-NOTEBOOK.md section 10.8): Phase 6
    // added several more real, multi-second waits on top of what Phases
    // 1-5 already needed (two separate real reciprocal-metric
    // kretschmann computations, one deliberately left to finish on its
    // own rather than being cancelled, plus several bounded 300-500ms
    // waits for refusal handlers to settle) -- 30s, enough before this
    // phase existed, was found too tight once it was added.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !report_path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for keytest.js to report back at {report_path:?} -- did the window even open?");
        std::thread::sleep(Duration::from_millis(100));
    }
    // The file can exist with a partial write mid-flight (std::fs::write
    // is not guaranteed atomic against a concurrent reader) -- keep
    // reading until it parses, bounded by the same deadline.
    let report: Report = loop {
        let mut contents = String::new();
        let opened = std::fs::File::open(&report_path).and_then(|mut f| f.read_to_string(&mut contents));
        if let Ok(_) = opened {
            if let Ok(r) = serde_json::from_str(&contents) {
                break r;
            }
        }
        assert!(Instant::now() < deadline, "report file at {report_path:?} never became valid JSON");
        std::thread::sleep(Duration::from_millis(100));
    };

    let failed: Vec<&Phase> = report.phases.iter().filter(|p| !p.ok).collect();
    assert!(
        report.ok && failed.is_empty(),
        "UI keymap test failed -- {} of {} phases failed:\n{}",
        failed.len(),
        report.phases.len(),
        failed.iter().map(|p| format!("  {}: {}", p.phase, p.details)).collect::<Vec<_>>().join("\n")
    );

    // Spell out the acceptance criteria explicitly here too, not only
    // inside keytest.js -- a future refactor of the JS driver that
    // silently dropped one of these phase names should fail this Rust
    // test loudly, not just report fewer (still all-green) phases.
    let phase_names: Vec<&str> = report.phases.iter().map(|p| p.phase.as_str()).collect();
    for expected in [
        "auto_focus_on_load",
        "all_four_blocks_executed_in_order",
        "execution_counts_are_sequential_and_unique",
        "kretschmann_output_in_dom",
        "kretschmann_gutter_shows_a_real_number",
        "fifth_block_created",
        "fifth_block_has_real_focus",
        "fifth_block_has_no_execution_count",
        "fifth_gutter_shows_empty_marker",
        "ctrl_enter_creates_nothing",
        "ctrl_enter_keeps_focus_on_same_block",
        "alt_enter_inserts_between_existing_blocks",
        "alt_enter_focuses_new_block",
        // Etapa 3b, "obsolescência visível" (DESIGN-NOTEBOOK.md section
        // 9) -- see keytest.js's own Phase 4 comment for why setup
        // there uses `invoke` for create/edit (this phase is about the
        // display of obsolescence, not keymap/focus wiring, already
        // covered by the phases above) while execute/delete still go
        // through real dispatched events.
        "obsolescence_setup_all_four_executed_and_not_obsolete",
        "edit_marks_self_and_everything_below_obsolete",
        "edit_triggers_no_execution_output_byte_identical",
        "obsolete_block_shows_amber_class_and_label_in_dom",
        "reexecuting_clears_only_that_blocks_own_mark",
        "undo_setup_edit_marks_it_obsolete",
        "undoing_edit_back_to_executed_text_clears_the_mark",
        "undo_did_not_change_the_result",
        "deleting_marks_only_executed_blocks_below_it_obsolete",
        "a_never_executed_block_cannot_be_obsolete",
        "never_executed_block_gutter_shows_empty_marker_not_amber",
        // Etapa 3b, segunda parte, "exclusão mútua global entre blocos"
        // (DESIGN-NOTEBOOK.md section 10.8) -- acceptance criteria
        // (a)-(e) from that decision, each with its own phase name so a
        // future refactor of keytest.js's Phase 6 that silently dropped
        // one still fails this test loudly.
        "shift_enter_on_a_different_block_does_not_start_execution_while_one_is_running", // (a)
        // (a) -- the refusal itself has to be a perceptible EDGE, not
        // just correct final text: the status bar's busy message is
        // ambient and unchanged by a refusal, so a check against only
        // the final text would pass even for a silently-swallowed
        // keypress. These two assert the actual before/after
        // transition (`dataset.refusalPulse`, notebook.js's
        // `flashRefusal`) on the status bar and on the specific block
        // occupying execution.
        "the_refusal_produces_a_status_bar_event_distinct_from_the_unchanged_ambient_text",
        "the_refusal_also_flashes_the_specific_block_that_is_occupying_execution",
        "the_status_bar_text_also_correctly_names_the_busy_block_by_number",              // (a)
        "alt_enter_on_a_different_block_is_also_refused_while_one_is_running",             // (a)
        "ctrl_enter_on_a_different_block_is_also_refused_while_one_is_running",             // (a)
        "refused_block_execution_count_stays_unchanged_across_every_refusal",              // (b)
        "refused_block_keeps_its_previous_output_byte_identical",                          // (b)
        "cancelling_the_running_block_unblocks_the_previously_refused_block",               // (c)
        "the_running_block_finishing_naturally_also_unblocks_the_previously_refused_block", // (d)
        "editing_a_different_block_still_works_while_one_is_running",                       // (e)
        "focusing_a_different_block_still_works_while_one_is_running",                      // (e)
        // Etapa 3, expression aliases (NOME := EXPR) -- Phase 7.
        "alias_declared_in_one_block_and_used_in_a_later_block_produces_a_real_result",
        "alias_expansion_matches_the_same_expression_written_out_by_hand_byte_for_byte",
        "editing_the_alias_block_marks_the_metric_block_that_uses_it_obsolete",
        "editing_the_alias_block_marks_the_query_block_that_depends_on_it_obsolete",
        "reexecuting_after_an_alias_change_clears_the_obsolete_mark_and_uses_the_new_value",
        // Etapa 3, tensor-result typography + click-to-copy LaTeX -- Phase 8.
        "riemann_from_metric_shows_named_comma_free_covariant_indices",
        "no_component_formula_uses_raw_comma_separated_integer_indices",
        "no_component_index_notation_contains_an_upper_index_caret_riemann_from_metric_is_fully_covariant",
        "no_component_formula_string_contains_the_orbit_note_text",
        "dom_shows_one_component_row_per_shown_component_each_with_real_katex_output",
        "no_katex_rendered_node_in_the_dom_contains_the_mangled_prose_this_bug_used_to_produce",
        "clicking_a_component_copies_exactly_that_components_clean_latex_to_the_real_os_clipboard",
        "the_copied_text_never_includes_the_orbit_note_or_any_other_components_formula",
        "clicking_a_component_shows_a_brief_non_modal_copied_confirmation_in_the_dom",
        // Marco 6 step 4, indeterminate functions (f(r)/f'(r)) -- Phase 9.
        "a_metric_with_an_unknown_f_of_r_is_accepted_not_rejected_as_unknown_function",
        "christoffel_of_an_unknown_f_of_r_produces_at_least_one_component",
        "some_christoffel_component_contains_f_of_r_itself",
        "some_christoffel_component_contains_f_prime_of_r",
        "dom_shows_real_katex_output_for_the_indeterminate_function_components",
        // Marco 6 step 4, round B, the geodesic equation -- Phase 10.
        "geodesic_of_schwarzschild_is_accepted_and_produces_a_result",
        "geodesic_produces_exactly_four_equations_one_per_coordinate",
        "geodesic_latex_uses_dot_notation_for_the_acceleration_and_velocity_terms",
        "geodesic_latex_never_leaks_the_affine_parameter_name_under_dot_notation",
        "dom_shows_one_real_katex_row_per_geodesic_equation",
        // Marco 6 step 4, round C, the geodesic equation solved for the
        // acceleration -- Phase 11.
        "accel_of_schwarzschild_is_accepted_and_produces_a_result",
        "accel_produces_exactly_four_solved_equations_one_per_coordinate",
        "accel_latex_shows_dot_notation_on_both_sides_of_the_equals_sign",
        "accel_never_shows_the_canonical_equals_zero_form",
        "dom_shows_one_real_katex_row_per_solved_accel_equation",
        // Marco 6 step 5, the Einstein tensor -- Phase 12.
        "einstein_of_reissner_nordstrom_is_accepted_and_produces_a_result",
        "einstein_of_reissner_nordstrom_produces_real_nonzero_components",
        "einstein_component_uses_the_real_g_label_and_named_indices",
        "dom_shows_one_real_katex_row_per_einstein_component",
        "einstein_of_schwarzschild_shows_no_component_rows_all_ten_are_zero",
        "einstein_of_schwarzschild_summary_names_ten_identically_zero_components",
        // Marco 6 step 6, curvature invariants and the Weyl tensor -- Phase 13.
        "weyl_of_reissner_nordstrom_is_accepted_and_produces_a_result",
        "weyl_of_reissner_nordstrom_produces_real_nonzero_components",
        "weyl_component_uses_the_real_c_label_and_named_indices",
        "dom_shows_one_real_katex_row_per_weyl_component",
        "riccisquare_of_reissner_nordstrom_is_accepted_and_produces_a_result",
        "dom_shows_real_katex_output_for_riccisquare",
        // Rodada Galeria, `load` -- Phase 14.
        "gallery_panel_lists_real_entries",
        "gallery_panel_has_an_frw_row",
        "gallery_panel_closes_after_picking_an_entry",
        "load_pastes_exactly_the_gallery_entrys_own_declaration_blocks",
        "loaded_blocks_start_never_run_editable_text_not_opaque",
        "loaded_frw_metric_computes_a_real_scalar_showing_the_scale_factor",
        // Botões "Limpar execução" / "Novo caderno" -- Phase 15.
        "clear_execution_setup_block_has_a_real_execution_count_before_clearing",
        "clear_execution_button_resets_every_gutter_to_empty_and_keeps_source_text",
        "clear_execution_resets_every_output_to_never_run",
        "clear_execution_reexecuting_shows_bracket_one_not_continued_numbering",
        "new_notebook_confirm_panel_opens_on_click_without_destroying_anything",
        "opening_the_new_notebook_confirmation_does_not_touch_any_block",
        "new_notebook_cancel_closes_the_panel_and_leaves_every_block_untouched",
        "escape_on_the_new_notebook_confirmation_also_only_cancels",
        "new_notebook_confirm_produces_a_single_blank_block",
        "new_notebook_confirm_panel_closes_after_confirming",
    ] {
        assert!(phase_names.contains(&expected), "expected keytest.js to report a '{expected}' phase, got: {phase_names:?}");
    }
}
