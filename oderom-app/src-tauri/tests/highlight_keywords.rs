//! Guards against the exact bug a user found in the notebook window:
//! `export sympy kretschmann` rendered with no syntax highlighting at
//! all, because `dist/oderom-mode.js`'s keyword list was a
//! hand-maintained copy of `oderom_cli::parser`'s grammar that nobody
//! updated when `export` was added -- the same class of bug that had
//! already happened once before, for the `sin`/`cos`/`exp`-family
//! functions.
//!
//! `oderom-app/src-tauri/build.rs` now regenerates `dist/oderom-mode.js`
//! from the parser's own keyword arrays on every real build, which is
//! the actual fix (see that file's own doc comment) -- this test is the
//! belt-and-suspenders fallback for the case the user explicitly asked
//! for: something that fails loudly if the checked-in file and the
//! parser's real keyword set are ever allowed to disagree (a stale
//! `dist/oderom-mode.js` committed without rebuilding `oderom-app`, or
//! a hand-edit made directly to the generated file). No real window,
//! no display needed -- unlike `tests/keymap.rs`, this only reads two
//! files and calls into `oderom_cli::parser` directly.

include!("../build_support.rs");

fn dist_oderom_mode_js_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/oderom-mode.js")
}

#[test]
fn dist_oderom_mode_js_matches_what_the_parsers_own_keywords_generate() {
    let template = include_str!("../oderom-mode.js.template");
    let expected = generate_oderom_mode_js(template);
    let path = dist_oderom_mode_js_path();
    let actual = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    assert_eq!(
        actual, expected,
        "dist/oderom-mode.js is out of sync with oderom_cli::parser's own keyword lists -- \
         rebuild oderom-app (its build.rs regenerates this file automatically from the parser) \
         rather than hand-editing dist/oderom-mode.js"
    );
}

/// The specific regression this test suite exists for, named
/// explicitly rather than left to the general diff above alone: every
/// query keyword the parser accepts -- including `export`, the one
/// that actually went missing -- appears in the generated highlighter.
#[test]
fn every_command_keyword_the_parser_accepts_is_present_in_the_generated_highlighter() {
    let template = include_str!("../oderom-mode.js.template");
    let js = generate_oderom_mode_js(template);
    for word in oderom_cli::parser::CommandName::keywords() {
        assert!(js.contains(word), "command keyword `{word}` (accepted by CommandName::from_str) is missing from the generated highlighter:\n{js}");
    }
    assert!(js.contains(oderom_cli::parser::EXPORT_KEYWORD), "the `export` keyword is missing from the generated highlighter:\n{js}");
    for word in oderom_cli::parser::export_target_keywords() {
        assert!(js.contains(word), "export target keyword `{word}` is missing from the generated highlighter:\n{js}");
    }
    for word in [oderom_cli::parser::VARIANCE_UP, oderom_cli::parser::VARIANCE_DOWN] {
        assert!(js.contains(word), "variance marker keyword `{word}` is missing from the generated highlighter:\n{js}");
    }
    for word in oderom_cli::parser::DECLARATION_KEYWORDS {
        assert!(js.contains(word), "declaration keyword `{word}` is missing from the generated highlighter:\n{js}");
    }
}
