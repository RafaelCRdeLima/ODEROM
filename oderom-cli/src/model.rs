//! What a `.od` file parses into: [`oderom_core::Registry`] (Marco 1's
//! abstract algebra -- manifolds, bundles, heads) plus the Marco 2-level
//! concrete objects the UI CLI adds (DESIGN-UI.md section 6) -- named
//! charts, metrics (a head plus its component values in one chart), and
//! connections (a bare Christoffel `Grid`, no metric required). One
//! struct, because a `.od` file is one language, not several -- see
//! DESIGN-UI.md 6.1.

use oderom_components::{Chart, ComponentTensor, Grid};
use oderom_core::{HeadId, Registry};
use oderom_expr::Expr;
use std::collections::HashMap;

/// `Clone` exists for `oderom-session`'s `Session::begin_query` (Etapa
/// 3b, cancellation): a query runs on its own thread against an owned
/// snapshot of the current `Model`, never a borrow tied to `Session`'s
/// own lifetime, so `Session` stays free for other callers (other
/// blocks' edits, `list_blocks`, ...) while that thread runs.
/// Registry/Chart/ComponentTensor/Grid all already derive `Clone`; nothing
/// here is deep or expensive to duplicate (the actual expensive
/// intermediate values -- `Expr`/`Grid` computed *during* a query --
/// live in `ComputeCache`, not here).
#[derive(Default, Clone)]
pub struct Model {
    pub registry: Registry,
    pub charts: HashMap<String, Chart>,
    /// Chart name, head, and component values, keyed by the metric's
    /// declared name.
    pub metrics: HashMap<String, (String, HeadId, ComponentTensor)>,
    /// Chart name and the raw Γ, keyed by the connection's declared name.
    pub connections: HashMap<String, (String, Grid)>,
    /// Expression aliases (`NOME := EXPR`), keyed by name -- each value
    /// is already fully expanded (any alias it itself referenced is
    /// already inlined) by the time it lands here, so nothing that reads
    /// this map ever needs to know an alias was involved. Pure syntactic
    /// sugar, not a new kind of declared object the rest of the crate
    /// has to understand: nothing outside `parser.rs`/`expr_parser.rs`
    /// ever reads this field today.
    pub aliases: HashMap<String, Expr>,
}

impl Model {
    pub fn new() -> Self {
        Model::default()
    }
}
