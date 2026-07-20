//! What a `.od` file parses into: [`oderom_core::Registry`] (Marco 1's
//! abstract algebra -- manifolds, bundles, heads) plus the Marco 2-level
//! concrete objects the UI CLI adds (DESIGN-UI.md section 6) -- named
//! charts, metrics (a head plus its component values in one chart), and
//! connections (a bare Christoffel `Grid`, no metric required). One
//! struct, because a `.od` file is one language, not several -- see
//! DESIGN-UI.md 6.1.

use oderom_components::{Chart, ComponentTensor, Grid};
use oderom_core::{HeadId, Registry};
use std::collections::HashMap;

#[derive(Default)]
pub struct Model {
    pub registry: Registry,
    pub charts: HashMap<String, Chart>,
    /// Chart name, head, and component values, keyed by the metric's
    /// declared name.
    pub metrics: HashMap<String, (String, HeadId, ComponentTensor)>,
    /// Chart name and the raw Γ, keyed by the connection's declared name.
    pub connections: HashMap<String, (String, Grid)>,
}

impl Model {
    pub fn new() -> Self {
        Model::default()
    }
}
