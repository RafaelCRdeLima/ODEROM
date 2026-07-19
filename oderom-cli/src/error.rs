use thiserror::Error;

/// Errors surfaced by the `oderom` binary, printed to stderr.
#[derive(Error, Debug)]
pub enum CliError {
    #[error("could not read `{path}`: {source}")]
    Io { path: String, source: std::io::Error },

    #[error("parse error: {0}")]
    Parse(String),

    #[error(transparent)]
    Core(#[from] oderom_core::CoreError),

    #[error(transparent)]
    Canon(#[from] oderom_canon::CanonError),

    #[error("usage: oderom canon [--prelude PATH] \"<expression>\"")]
    Usage,
}
