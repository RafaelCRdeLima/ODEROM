//! `oderom-cli` as a library: the `.od` document model, parser (both
//! grammars -- declarations and worksheet `Query`s, `parser.rs`), and
//! error type, reusable by anything that needs to load a `.od` file or
//! parse a worksheet entry without shelling out to the `oderom` binary.
//!
//! DESIGN-UI-SESSION.md section 6: the future `oderom-session` crate
//! (no UI, testable headless) depends on this crate's library target,
//! not the binary -- direct function calls, no subprocess, no IPC. The
//! `oderom` binary (`main.rs`) is unchanged in behavior; it now calls
//! into this library instead of declaring the same modules privately.

pub mod commands;
pub mod error;
pub mod gallery;
pub mod model;
pub mod parser;
pub mod span;

mod expr_parser;
mod index_resolve;

pub use error::CliError;
