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

    #[error(transparent)]
    Component(#[from] oderom_components::ComponentError),

    #[error("no metric or connection found in the file")]
    NoMetricOrConnection,

    #[error("the file declares more than one {kind} ({names}); pick one with --{kind}")]
    AmbiguousChoice { kind: &'static str, names: String },

    #[error("`{name}` needs a metric to invert (only a connection was declared)")]
    NeedsMetric { name: String },

    #[error("expression exceeded {limit} nodes ({nodes}) at stage `{stage}`")]
    NodeLimitExceeded { stage: String, nodes: usize, limit: usize },

    #[error("timed out after {timeout:?} -- last stage in progress: `{stage}`")]
    Timeout { stage: String, timeout: std::time::Duration },

    #[error(
        "usage: oderom canon [--prelude PATH] \"<expression>\"\n   or: oderom {{christoffel|riemann|ricci|scalar|kretschmann}} FILE [--metric NAME | --connection NAME] [--target unicode|latex|json] [--max-lines N] [--max-nodes N] [--timeout SECONDS]"
    )]
    Usage,
}
