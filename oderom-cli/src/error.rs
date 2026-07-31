use crate::span::Position;
use thiserror::Error;

/// Errors surfaced by the `oderom` binary, printed to stderr.
#[derive(Error, Debug)]
pub enum CliError {
    #[error("could not read `{path}`: {source}")]
    Io { path: String, source: std::io::Error },

    /// `position` is `None` only for errors that are not about a source
    /// location at all (e.g. `resolve_choice`'s "no metric named X" --
    /// a CLI-flag/name-resolution error raised after parsing has
    /// already finished, with no token in flight to point at). Every
    /// error raised *during* lexing/parsing (`TokStream::error`, the
    /// only place besides those few post-parse sites that constructs
    /// this variant) always supplies `Some` -- never a guessed or
    /// stale position.
    #[error("parse error: {message}{}", position.map(|p| format!(" (line {}, column {})", p.line, p.column)).unwrap_or_default())]
    Parse { message: String, position: Option<Position> },

    #[error(transparent)]
    Core(#[from] oderom_core::CoreError),

    #[error(transparent)]
    Canon(#[from] oderom_canon::CanonError),

    #[error(transparent)]
    Component(#[from] oderom_components::ComponentError),

    #[error("no metric or connection found in the file")]
    NoMetricOrConnection,

    #[error(transparent)]
    UnknownGalleryEntry(#[from] crate::gallery::UnknownGalleryEntry),

    #[error("the file declares more than one {kind} ({names}); pick one with --{kind}")]
    AmbiguousChoice { kind: &'static str, names: String },

    #[error("`{name}` needs a metric to invert (only a connection was declared)")]
    NeedsMetric { name: String },

    /// An index-variance marker (`christoffel [up,down,down]`, Rodada
    /// Variancia) on Christoffel specifically -- refused unconditionally,
    /// regardless of the marker's own length or content, because the
    /// question it asks has no answer: Christoffel is not a tensor, it
    /// transforms with a non-homogeneous term (that is exactly what
    /// makes it a connection rather than a tensor), so raising or
    /// lowering its indices with the metric would produce an array of
    /// numbers that is not a tensor, not a connection, not any geometric
    /// object at all -- a plausible-looking but meaningless result, the
    /// one class of output this project refuses to produce even when it
    /// could compute *something*. `christoffel` with no marker is
    /// completely unaffected -- it keeps working in its one natural
    /// mixed form (`Gamma^a_bc`) exactly as before; only a variance
    /// change is barred.
    #[error("Christoffel is not a tensor -- it transforms with a non-homogeneous term, which is exactly what makes it a connection instead of a tensor. Raising or lowering its indices with the metric would not produce a tensor, a connection, or any geometric object -- it would just be numbers with no meaning. For curvature, use `riemann` instead.")]
    ChristoffelNotATensor,

    /// An index-variance marker on a bare scalar query (`scalar`,
    /// `kretschmann`, `riccisquare`, `gaussbonnet`, `weylsquare`) --
    /// these have no indices at all to raise or lower.
    #[error("`{name}` is a scalar -- it has no indices, so an index-variance marker doesn't apply. Use it on riemann, ricci, einstein, or weyl instead.")]
    VarianceOnScalar { name: String },

    /// An index-variance marker whose length doesn't match the named
    /// tensor's own rank (`riemann [up,down,down]` -- three entries for
    /// a rank-4 tensor). Checked before any raising/lowering is
    /// attempted -- never silently padded, truncated, or guessed.
    #[error("`{command}` has {expected} index/indices, but the variance marker names {found} -- they must match exactly")]
    VarianceArityMismatch { command: String, expected: usize, found: usize },

    #[error("expression exceeded {limit} nodes ({nodes}) at stage `{stage}`")]
    NodeLimitExceeded { stage: String, nodes: usize, limit: usize },

    #[error("denominator degree exceeded {limit} ({degree}) at stage `{stage}`")]
    DenominatorDegreeExceeded { stage: String, degree: i32, limit: i32 },

    #[error("timed out after {timeout:?} -- last stage in progress: `{stage}`")]
    Timeout { stage: String, timeout: std::time::Duration },

    #[error(
        "usage: oderom canon [--prelude PATH] \"<expression>\"\n   or: oderom simplify [--prelude PATH] [--metric HEAD]... [--bianchi HEAD]... \"<sum of monomials>\"  (abstract indices; T[a,b;c] is a covariant derivative; --metric contracts a declared metric away (index raising/lowering), --bianchi declares the first Bianchi identity -- both declared, never inferred)\n   or: oderom {{christoffel|riemann|ricci|scalar|kretschmann|einstein|riccisquare|gaussbonnet|weyl|weylsquare}} FILE [--metric NAME | --connection NAME] [--target unicode|latex|json|mathematica|sympy] [--max-lines N] [--max-nodes N] [--max-denominator-degree N] [--timeout SECONDS]\n   or: oderom {{geodesic|accel}} FILE --param NAME [--metric NAME | --connection NAME] [--target unicode|latex|json|mathematica|sympy] [--max-lines N] [--max-nodes N] [--max-denominator-degree N] [--timeout SECONDS]\n   or: oderom export {{mathematica|sympy}} {{christoffel|riemann|ricci|scalar|kretschmann|geodesic|accel|einstein|riccisquare|gaussbonnet|weyl|weylsquare}} FILE [--metric NAME | --connection NAME] [--param NAME] [--max-nodes N] [--max-denominator-degree N] [--timeout SECONDS]\n   or: oderom load NAME  (prints a known gallery metric's declarations to stdout; an unknown NAME lists every known one)"
    )]
    Usage,
}
