//! `oderom-components` -- tensor components in a coordinate chart, the
//! curvature quantities computed from them (Marco 2), multi-chart
//! atlases related by transition maps (Marco 3), numerical geodesic
//! integration / parallel transport (Marco 5), and symmetry-aware
//! display of components (UI Camada A, `render`).
//!
//! A chart's components are symbolic ([`oderom_expr::Expr`]), not
//! numbers -- a tensor's component is a function of the chart's
//! coordinates. [`tensor::ComponentTensor`] stores only the independent
//! components, one per orbit of a `TensorHead`'s declared symmetry
//! group -- reusing Marco 1's [`oderom_core::Bsgs`] rather than
//! inventing a second mechanism for "which components are related by
//! symmetry". See `curvature` for Christoffel/Riemann/Ricci (the one
//! real restriction there is diagonal metrics only), `atlas` for
//! multi-chart transitions, `holonomy` for the numerical part -- the
//! only place in this project that evaluates rather than reasons about
//! tensors exactly -- and `render` for turning a
//! `ComponentTensor`/`Grid` into readable output by exploiting that same
//! symmetry group to show only independent components.

pub mod atlas;
pub mod chart;
pub mod curvature;
pub mod error;
pub mod grid;
pub mod holonomy;
pub mod render;
pub mod tensor;

pub use atlas::{metric_agrees_across_transition, Atlas, ChartId, TransitionMap};
pub use chart::Chart;
pub use error::ComponentError;
pub use grid::Grid;
pub use holonomy::{ChristoffelPrograms, integrate_geodesic_with_transport};
pub use render::{classify_grid, classify_tensor, render_classes, ComponentClass};
pub use tensor::ComponentTensor;
