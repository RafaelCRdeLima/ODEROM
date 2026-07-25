//! Source positions and spans, shared by the `.od` document grammar
//! (`parser.rs`) and the worksheet-entry grammar (`Query`, same module)
//! -- one type, used by both, not duplicated per grammar (DESIGN-UI-
//! SESSION.md: the two are one grammar with two entry points, and this
//! is exactly the kind of shared piece that claim rests on).
//!
//! 1-based line and column, matching what every text editor (including
//! the one this exists for) already shows a human.

/// A single point in source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// A half-open range `[start, end)` in source text -- what a token
/// occupies, or what an AST node's own tokens span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

/// A parsed value plus the span of source text it came from -- every
/// `Query` AST node field that a future editor might want to
/// underline/highlight carries one of these, not a bare value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Self {
        Spanned { value, span }
    }
}
