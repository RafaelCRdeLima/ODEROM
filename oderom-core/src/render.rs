//! A trait for producing output in more than one target format, so "how
//! do I print this" is decided once per type instead of reinvented ad
//! hoc in each crate (see DESIGN-UI.md, Camada A). `Display`, where a
//! type has one, is a thin wrapper over `render(Target::Unicode)` --
//! never an independent implementation that could drift from it.
//!
//! This lives in `oderom-core` rather than `oderom-expr` (where the
//! first real consumer, `Expr`, lives) because every other crate in the
//! workspace already depends on `oderom-core`; a type here is reusable
//! by the scalar CAS, the abstract monomial/e-graph layers, and the
//! chart-level tensor renderer without any of them depending on each
//! other.

/// Which output format [`Render::render`] should produce.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Target {
    /// Plain human-readable text for a terminal.
    Unicode,
    /// LaTeX math-mode source. Not an optional extra alongside Unicode --
    /// typeset output is why this project exists.
    Latex,
    /// A hand-written structural encoding (no `serde`: the grammar here
    /// is small and fixed, and the project's dependency policy asks
    /// before adding parsing/serialization libraries). Exists now, ahead
    /// of any concrete consumer, because it is the wire format a future
    /// Jupyter-kernel front end (the current working hypothesis for a
    /// UI, see DESIGN-UI.md) would need -- keeping that door open is
    /// cheaper than reopening it later.
    Json,
}

/// Something that can render itself in any [`Target`] format.
pub trait Render {
    fn render(&self, target: Target) -> String;
}
