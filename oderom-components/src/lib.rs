//! `oderom-components` -- Marco 2: tensor components in a coordinate
//! chart, and the curvature quantities computed from them.
//!
//! Marco 2 has one chart per manifold (no atlas -- Marco 3) and a
//! symbolic scalar per component ([`oderom_expr::Expr`]), not a number.
//! [`tensor::ComponentTensor`] stores only the independent components,
//! one per orbit of a `TensorHead`'s declared symmetry group -- reusing
//! Marco 1's [`oderom_core::Bsgs`] rather than inventing a second
//! mechanism for "which components are related by symmetry". See
//! `curvature` for Christoffel/Riemann/Ricci and the module docs there
//! for the one real restriction (diagonal metrics only).

pub mod chart;
pub mod curvature;
pub mod error;
pub mod grid;
pub mod tensor;

pub use chart::Chart;
pub use error::ComponentError;
pub use grid::Grid;
pub use tensor::ComponentTensor;
