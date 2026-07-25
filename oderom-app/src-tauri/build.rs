//! Besides the ordinary `tauri_build::build()` step, this regenerates
//! `dist/oderom-mode.js` (the CodeMirror syntax-highlighting mode) from
//! `oderom_cli::parser`'s own keyword lists on every build -- see
//! `generate_oderom_mode_js_file`'s own doc comment for why.

include!("build_support.rs");

fn main() {
    generate_oderom_mode_js_file();
    tauri_build::build()
}

/// The highlighter used to be a hand-maintained copy of the grammar's
/// keyword lists, and it had already gone stale twice: once for the
/// `sin`/`cos`/`exp`-family functions, once for `export` itself (found
/// by a user noticing `export sympy kretschmann` rendered with no
/// color at all while every other command did). Both times the actual
/// bug was structural, not a one-off typo -- CodeMirror's mode is a
/// plain JS regex with no connection whatsoever to
/// `oderom_cli::parser`, so nothing forced the two to be edited
/// together.
///
/// This closes that gap by generating `dist/oderom-mode.js` here, at
/// build time, straight from the parser's own canonical arrays via
/// `generate_oderom_mode_js` (`build_support.rs`, shared with
/// `tests/highlight_keywords.rs`'s own drift check) -- a NEW query
/// command added to `CommandName` tomorrow appears in the highlighter
/// the next time this crate is built, with no second edit required
/// anywhere. The file this writes is still checked into `dist/` (Tauri's
/// `frontendDist` embeds it as a plain static asset, same as every
/// other file there) -- it is just no longer hand-edited; running
/// `cargo build` on this crate keeps it correct.
fn generate_oderom_mode_js_file() {
    let template = include_str!("oderom-mode.js.template");
    let js = generate_oderom_mode_js(template);

    let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/oderom-mode.js");
    std::fs::write(&out_path, js).unwrap_or_else(|e| panic!("failed to write {}: {e}", out_path.display()));

    // Rebuild whenever the parser's own keyword source or this
    // generator's template changes -- Cargo's default "rerun if
    // anything in this crate changed" already covers build.rs/
    // build_support.rs themselves.
    println!("cargo:rerun-if-changed=oderom-mode.js.template");
    println!("cargo:rerun-if-changed=../../oderom-cli/src/parser.rs");
}
