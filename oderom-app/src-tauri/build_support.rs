// Shared between `build.rs` (which writes `dist/oderom-mode.js` from
// this on every real build) and `tests/highlight_keywords.rs` (which
// independently re-derives the same text and diffs it against whatever
// is actually checked into `dist/`, catching the case where the file
// went stale without a rebuild -- see `build.rs`'s own doc comment for
// why this exists at all). `include!`d into both rather than a normal
// module: `build.rs` cannot depend on its own crate's lib target (it
// runs before that target compiles), so this is the standard way two
// otherwise-separate compilation units share one piece of logic without
// duplicating it.

/// Fills `template`'s three placeholders from `oderom_cli::parser`'s
/// own canonical keyword arrays -- see `oderom-app/src-tauri/build.rs`'s
/// own doc comment for the full reasoning and for which six structural
/// words are deliberately NOT derived this way.
fn generate_oderom_mode_js(template: &str) -> String {
    let declaration_words: Vec<&str> = oderom_cli::parser::DECLARATION_KEYWORDS.to_vec();
    let auxiliary_words = ["on", "dim", "coords", "symmetry", "antisymmetric", "symmetric"];
    let declaration_group = declaration_words.iter().copied().chain(auxiliary_words).collect::<Vec<_>>().join("|");

    let command_group = oderom_cli::parser::CommandName::keywords().collect::<Vec<_>>().join("|");

    let modifier_words: Vec<&str> = std::iter::once(oderom_cli::parser::EXPORT_KEYWORD)
        .chain(oderom_cli::parser::export_target_keywords())
        .chain([oderom_cli::parser::VARIANCE_UP, oderom_cli::parser::VARIANCE_DOWN])
        .collect();
    let modifier_group = modifier_words.join("|");

    template
        .replace("__DECLARATION_KEYWORDS__", &declaration_group)
        .replace("__COMMAND_KEYWORDS__", &command_group)
        .replace("__MODIFIER_KEYWORDS__", &modifier_group)
}
