//! The spacetime gallery: known metrics loadable by name (Rodada
//! Galeria). Every entry here doubles as an engine acceptance fixture
//! -- see each entry's own comment for the closed-form curvature
//! result it is checked against (`oderom-notebook/tests/gallery.rs`,
//! `oderom-components/tests/antidesitter.rs`, `.../frw.rs`). A metric
//! that could not be verified this way must not appear in [`ENTRIES`]:
//! "uma métrica sem teste de curvatura verificado não deve existir" is
//! the whole point of the round this module belongs to.
//!
//! Lives here, at the bottom of the dependency chain (`oderom-session`
//! and `oderom-notebook` both depend on `oderom-cli`, never the other
//! way around), so both the standalone binary's `oderom load NAME`
//! subcommand (`main.rs`) and the notebook's `load` (which pastes each
//! entry's declaration text as new, independently-editable blocks --
//! `oderom_notebook::Notebook::load_gallery_entry`, still exported as
//! `oderom_notebook::gallery` for convenience) share exactly one
//! catalog. Originally implemented notebook-only, on the reasoning that
//! the CLI has no "block" to paste into; moved down after the CLI was
//! pointed out to be a first-class interface in its own right (this
//! project's own manual treats it that way) that a GUI-only gallery
//! would leave with no access to these fixtures at all. The two
//! surfaces still do genuinely different things with the same data --
//! the notebook pastes editable blocks, the CLI prints text to stdout
//! (`render` below) -- neither is a lesser copy of the other.

/// One gallery entry: a name to load, human-facing text for a picker
/// UI/the manual, and the block-by-block declaration text. In a
/// notebook, `blocks[i]` becomes one new block, in order; on the CLI,
/// [`render`] joins them into one `.od` file's worth of text.
pub struct GalleryEntry {
    /// The identifier `load` matches against, e.g. `load desitter`.
    pub name: &'static str,
    /// Short display title for a picker UI and the manual.
    pub title: &'static str,
    /// One line: what this spacetime is.
    pub description: &'static str,
    /// One line: the characterizing invariant this entry is verified
    /// against, for the manual and the UI.
    pub invariant: &'static str,
    pub blocks: &'static [&'static str],
}

const DIM4: &str = "manifold M dim 4\nbundle TM on M dim 4";

pub const ENTRIES: &[GalleryEntry] = &[
    // Constant curvature, `R = 12 H^2` -- maximally symmetric, Weyl
    // identically zero. Reuses the exact metric
    // `oderom-components/tests/desitter.rs` already carries as its own
    // acceptance fixture (`ds^2 = -dt^2 + exp(2Ht)(dx^2+dy^2+dz^2)`).
    GalleryEntry {
        name: "desitter",
        title: "De Sitter",
        description: "Curvatura constante positiva, fatiamento plano (FRW com a(t) = exp(Ht))",
        invariant: "R = 12 H^2 (constante); Weyl identicamente nulo",
        blocks: &[
            DIM4,
            "chart flat on M coords (t, x, y, z)",
            "metric g on flat bundle TM {\n  [t,t] = -1,\n  [x,x] = exp(2*H*t),\n  [y,y] = exp(2*H*t),\n  [z,z] = exp(2*H*t)\n}",
        ],
    },
    // Constant curvature, `R = -12 H^2` -- the negative-curvature
    // counterpart, Poincare-patch coordinates (`ds^2 = (1/(Hz))^2
    // (-dt^2+dx^2+dy^2+dz^2)`), a genuinely different closed form from
    // de Sitter's (a power-law conformal factor in a spatial coordinate,
    // not an exponential in time) -- not merely "de Sitter with H -> iH".
    GalleryEntry {
        name: "antidesitter",
        title: "Anti-de Sitter",
        description: "Curvatura constante negativa, coordenadas de Poincare",
        invariant: "R = -12 H^2 (constante); R_ab = -3 H^2 g_ab",
        blocks: &[
            DIM4,
            "chart poincare on M coords (t, x, y, z)",
            "metric g on poincare bundle TM {\n  [t,t] = -1/(H^2*z^2),\n  [x,x] = 1/(H^2*z^2),\n  [y,y] = 1/(H^2*z^2),\n  [z,z] = 1/(H^2*z^2)\n}",
        ],
    },
    // Flat FRW with an unspecified scale factor `a(t)` -- exercises
    // coordinate-as-function directly in a metric component (not inside
    // a geodesic-derived quantity, the only context this machinery had
    // exercised before this round). Closed form: `R = 6(a''/a +
    // (a'/a)^2)`.
    GalleryEntry {
        name: "frw",
        title: "FRW plano",
        description: "Friedmann-Robertson-Walker espacialmente plano, fator de escala a(t) generico",
        invariant: "R = 6(a''(t)/a(t) + a'(t)^2/a(t)^2)",
        blocks: &[
            DIM4,
            "chart frw on M coords (t, x, y, z)",
            "metric g on frw bundle TM {\n  [t,t] = -1,\n  [x,x] = a(t)^2,\n  [y,y] = a(t)^2,\n  [z,z] = a(t)^2\n}",
        ],
    },
    // Vacuum, spherically symmetric -- this project's own long-standing
    // acceptance fixture (`oderom-components/tests/schwarzschild.rs`),
    // identical text to `oderom-cli/tests/fixtures/schwarzschild_ascii.od`.
    GalleryEntry {
        name: "schwarzschild",
        title: "Schwarzschild",
        description: "Vacuo, estatico, simetrico esferico",
        invariant: "R_ab = 0; Kretschmann = 48 M^2 / r^6",
        blocks: &[
            DIM4,
            "chart schw on M coords (t, r, theta, phi)",
            "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}",
        ],
    },
    // Electrovacuum, spherically symmetric, charged -- this project's
    // own `oderom-components/tests/reissner_nordstrom.rs` fixture,
    // identical text to `oderom-cli/tests/fixtures/reissner_nordstrom.od`.
    GalleryEntry {
        name: "reissnernordstrom",
        title: "Reissner-Nordstrom",
        description: "Simetrico esferico, carregado (nao e vacuo: R != 0)",
        invariant: "R = 0 mas R_ab != 0 (traco nulo, tensor de Ricci nao); Kretschmann fecha em M, Q, r",
        blocks: &[
            DIM4,
            "chart schw on M coords (t, r, theta, phi)",
            "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}",
        ],
    },
];

/// The entry named `name`, if any -- `load`'s own lookup, on both
/// surfaces.
pub fn find(name: &str) -> Option<&'static GalleryEntry> {
    ENTRIES.iter().find(|e| e.name == name)
}

/// `entry`'s declarations as one `.od` file's worth of text -- what
/// `oderom load NAME` prints to stdout: the same blocks a notebook
/// would paste separately, joined by a single blank line, so the
/// output is itself a valid, directly loadable `.od` file (`oderom
/// load desitter > desitter.od` then `oderom scalar desitter.od`
/// works unmodified).
pub fn render(entry: &GalleryEntry) -> String {
    entry.blocks.join("\n\n")
}

/// `load NOME` named a metric not in [`ENTRIES`].
#[derive(Debug)]
pub struct UnknownGalleryEntry {
    pub name: String,
}

impl std::fmt::Display for UnknownGalleryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let known: Vec<&str> = ENTRIES.iter().map(|e| e.name).collect();
        write!(f, "unknown gallery entry `{}` (known: {})", self.name, known.join(", "))
    }
}

impl std::error::Error for UnknownGalleryEntry {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_name_is_unique() {
        let mut names: Vec<&str> = ENTRIES.iter().map(|e| e.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate gallery entry name");
    }

    #[test]
    fn find_locates_every_declared_entry_by_its_own_name() {
        for entry in ENTRIES {
            assert!(find(entry.name).is_some(), "find() must locate {:?}", entry.name);
        }
    }

    #[test]
    fn find_reports_unknown_names_as_none() {
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn every_entry_has_at_least_a_declaration_and_a_metric_block() {
        for entry in ENTRIES {
            assert!(entry.blocks.len() >= 2, "{:?} has too few blocks: {:?}", entry.name, entry.blocks);
        }
    }

    #[test]
    fn render_joins_blocks_with_a_blank_line_and_stays_parseable() {
        for entry in ENTRIES {
            let text = render(entry);
            for block in entry.blocks {
                assert!(text.contains(*block), "{:?}: rendered text must contain block {:?} verbatim", entry.name, block);
            }
            assert!(crate::parser::parse_model(&text).is_ok(), "{:?}: rendered text must parse as a valid .od file: {text}", entry.name);
        }
    }
}
