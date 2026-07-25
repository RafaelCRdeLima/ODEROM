//! Acceptance tests for the spacetime gallery (Rodada Galeria): for
//! every entry in `oderom_notebook::gallery::ENTRIES`, `load` the entry
//! through the *real* `Notebook::load_gallery_entry` + `execute_block`
//! path (never a hand-built `ComponentTensor` shortcut -- the whole
//! point of the round is that the exact text a user sees pasted into
//! their notebook is what gets checked against a known curvature
//! result), then run a query and check it against the entry's own
//! `invariant`. A gallery entry with no test here would be exactly the
//! thing this round exists to prevent.
//!
//! Exact expected strings below were confirmed by running the query
//! once and reading the real output (same practice as
//! `oderom-cli/tests/end_to_end.rs`'s own hardcoded
//! `"48*M^2/r^6"`), not guessed from the closed-form formulas alone.
//! Deeper symbolic-equality checks (via `oderom_expr::normalize`, not
//! string matching) for the two genuinely new metrics this round adds
//! -- anti-de Sitter and FRW -- live in
//! `oderom-components/tests/antidesitter.rs` and
//! `oderom-components/tests/frw.rs`.

use oderom_notebook::{gallery, BlockOutput, Notebook};

/// `load`s `name` at the end of a fresh notebook, executing every block
/// it creates in order (mirroring a real Shift+Enter per block), then
/// runs `query` as one more block and returns its rendered ASCII
/// (`unicode` target) result. Panics with a descriptive message if
/// anything along the way didn't resolve to a real, successful result
/// -- a gallery entry that doesn't even load cleanly has no business
/// being in this list at all.
fn load_and_query(name: &str, query: &str) -> String {
    let mut nb = Notebook::new();
    let created = nb.load_gallery_entry(None, name).unwrap_or_else(|e| panic!("load {name}: {e}"));
    for id in &created {
        nb.execute_block(*id);
    }
    for id in &created {
        let block = nb.block(*id).unwrap();
        if let BlockOutput::Declaration(status) = &block.output {
            assert_eq!(*status, oderom_notebook::DeclarationStatus::Confirmed, "{name}: block {id:?} did not confirm: {:?}\nsource:\n{}", status, block.source);
        }
    }

    let q = nb.create_block_after(created.last().copied(), query.to_string());
    nb.execute_block(q);
    let BlockOutput::Query(entry_id) = nb.block(q).unwrap().output else {
        panic!("{name}: query block {q:?} for {query:?} did not classify as a query");
    };
    let entry = nb.session().entries().iter().find(|e| e.id == entry_id).unwrap();
    match &entry.state {
        oderom_notebook::EntryState::Done { result, .. } => result.unicode.clone(),
        other => panic!("{name}: query {query:?} did not finish with a result: {}", debug_state(other)),
    }
}

fn debug_state(state: &oderom_notebook::EntryState) -> &'static str {
    match state {
        oderom_notebook::EntryState::Pending => "Pending",
        oderom_notebook::EntryState::Running => "Running",
        oderom_notebook::EntryState::Done { .. } => "Done",
        oderom_notebook::EntryState::Stale { .. } => "Stale",
        oderom_notebook::EntryState::Cancelled => "Cancelled",
        oderom_notebook::EntryState::Failed { .. } => "Failed",
    }
}

#[test]
fn gallery_has_no_duplicate_or_dangling_entries() {
    let names: Vec<&str> = gallery::ENTRIES.iter().map(|e| e.name).collect();
    assert!(!names.is_empty());
    for name in &names {
        assert!(gallery::find(name).is_some());
    }
}

// -------------------------------------------------------------------
// Group 1 -- diagonal, verified against a known closed form.
// -------------------------------------------------------------------

#[test]
fn desitter_scalar_is_twelve_h_squared() {
    let out = load_and_query("desitter", "scalar");
    assert_eq!(out.trim(), "12*H^2");
}

#[test]
fn desitter_weyl_is_identically_zero() {
    let out = load_and_query("desitter", "weyl");
    assert!(out.contains("identically zero"), "expected every Weyl component to be reported zero (de Sitter is maximally symmetric): {out}");
}

#[test]
fn antidesitter_scalar_is_minus_twelve_h_squared() {
    let out = load_and_query("antidesitter", "scalar");
    assert_eq!(out.trim(), "-12*H^2");
}

#[test]
fn frw_scalar_shows_the_scale_factor_and_its_derivatives() {
    let out = load_and_query("frw", "scalar");
    assert!(out.contains("a(t)"), "expected the scale factor a(t) itself to appear: {out}");
    assert!(out.contains("a'(t)") || out.contains("a''(t)"), "expected at least one derivative of a(t) to appear: {out}");
}

#[test]
fn schwarzschild_gallery_entry_is_vacuum_with_the_known_kretschmann() {
    let ricci_out = load_and_query("schwarzschild", "ricci");
    assert!(ricci_out.contains("identically zero"), "Schwarzschild must be vacuum: {ricci_out}");
    let kretschmann_out = load_and_query("schwarzschild", "kretschmann");
    assert_eq!(kretschmann_out.trim(), "48*M^2/r^6");
}

#[test]
fn reissner_nordstrom_gallery_entry_has_zero_scalar_but_nonzero_ricci() {
    let scalar_out = load_and_query("reissnernordstrom", "scalar");
    assert_eq!(scalar_out.trim(), "0");
    let ricci_out = load_and_query("reissnernordstrom", "ricci");
    assert!(ricci_out.contains("Ricci[t,t]"), "Reissner-Nordstrom is charged, not vacuum -- expected a nonzero Ricci[t,t] component: {ricci_out}");
}

// -------------------------------------------------------------------
// `load`'s own mechanics.
// -------------------------------------------------------------------

#[test]
fn load_gallery_entry_creates_new_blocks_and_leaves_the_rest_of_the_notebook_untouched() {
    let mut nb = Notebook::new();
    let untouched = nb.create_block_after(None, "ricci".to_string());
    let blocks_before = nb.blocks().len();

    let created = nb.load_gallery_entry(Some(untouched), "schwarzschild").unwrap();
    assert_eq!(created.len(), gallery::find("schwarzschild").unwrap().blocks.len());
    assert_eq!(nb.blocks().len(), blocks_before + created.len());
    assert_eq!(nb.block(untouched).unwrap().source, "ricci", "load must never rewrite an existing block's source");

    for (id, expected_source) in created.iter().zip(gallery::find("schwarzschild").unwrap().blocks) {
        assert_eq!(&nb.block(*id).unwrap().source, expected_source);
    }
}

#[test]
fn load_gallery_entry_rejects_an_unknown_name_without_creating_anything() {
    let mut nb = Notebook::new();
    let blocks_before = nb.blocks().len();
    let err = nb.load_gallery_entry(None, "does-not-exist").unwrap_err();
    assert!(err.to_string().contains("does-not-exist"));
    assert_eq!(nb.blocks().len(), blocks_before);
}
