//! Minimal hand-written recursive-descent parsing for `prelude.od` and for
//! the tensor-expression argument to `oderom canon`. No parser-generator
//! or new dependency: the grammar is small enough that a lexer plus a
//! handful of `parse_*` functions is clearer than pulling in a library.

use crate::error::CliError;
use crate::index_resolve::parse_component_line;
use crate::model::Model;
use crate::span::{Position, Span, Spanned};
use oderom_components::{Chart, ComponentTensor, Grid};
use oderom_core::{
    AbstractIndex, Factor, HeadId, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId,
    SlotSig, Target, Variance,
};
use oderom_expr::normalize;
use smallvec::SmallVec;
use std::collections::HashMap;

// ---------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident(String),
    Int(u64),
    /// A backslash-prefixed identifier (`\frac`, `\theta`, `\sin`, ...) --
    /// the LaTeX-flavored front-end's alternate spelling for things the
    /// ASCII grammar writes as bare identifiers or symbols (see
    /// `expr_parser`). The name excludes the backslash.
    Command(String),
    Sym(char),
    Eof,
}

pub(crate) struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
    column: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer { chars: src.chars().peekable(), line: 1, column: 1 }
    }

    fn pos(&self) -> Position {
        Position { line: self.line, column: self.column }
    }

    /// The only place this lexer's position advances -- every character
    /// consumed anywhere below goes through this, so `next_tok`'s
    /// returned `Span` is never approximate.
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next();
        if let Some(c) = c {
            if c == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
        c
    }

    pub(crate) fn next_tok(&mut self) -> Result<(Tok, Span), CliError> {
        loop {
            match self.chars.peek() {
                None => {
                    let p = self.pos();
                    return Ok((Tok::Eof, Span { start: p, end: p }));
                }
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('#') => {
                    while let Some(&c) = self.chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
        let start = self.pos();
        let tok = match self.chars.peek().copied() {
            None => Tok::Eof,
            Some(c) if c.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(&d) = self.chars.peek() {
                    if d.is_ascii_digit() {
                        n.push(d);
                        self.bump();
                    } else {
                        break;
                    }
                }
                n.parse().map(Tok::Int).map_err(|_| CliError::Parse {
                    message: format!("bad integer `{n}`"),
                    position: Some(start),
                })?
            }
            // `_` is deliberately not an identifier character: it is the
            // LaTeX-flavored front-end's subscript operator (`g_{tt}`),
            // its own `Sym('_')` token, same as `^` for powers -- an
            // identifier ending right before a subscript (`g_{tt}`) must
            // lex as two tokens, not one `Ident("g_")`.
            Some(c) if c.is_alphabetic() => {
                let mut s = String::new();
                while let Some(&d) = self.chars.peek() {
                    if d.is_alphanumeric() {
                        s.push(d);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Tok::Ident(s)
            }
            Some('\\') => {
                self.bump();
                let mut s = String::new();
                while let Some(&d) = self.chars.peek() {
                    if d.is_alphabetic() {
                        s.push(d);
                        self.bump();
                    } else {
                        break;
                    }
                }
                if s.is_empty() {
                    return Err(CliError::Parse {
                        message: "`\\` must be followed by a command name".to_string(),
                        position: Some(start),
                    });
                }
                Tok::Command(s)
            }
            Some(c) => {
                self.bump();
                Tok::Sym(c)
            }
        };
        let end = self.pos();
        Ok((tok, Span { start, end }))
    }
}

/// Turns a source string into a token stream with one token of lookahead.
/// Each token's `Span` (start/end source position) is tracked alongside
/// it, never inside `Tok` itself -- `Tok` stays a plain value compared by
/// `==` everywhere it already is (`*toks.peek() == Tok::Sym(',')`, all
/// over this crate); baking a span into it would either break every one
/// of those comparisons or need a hand-rolled `PartialEq` that ignores
/// the span, more surprising than keeping the two separate.
pub(crate) struct TokStream {
    toks: Vec<Tok>,
    spans: Vec<Span>,
    pos: usize,
}

impl TokStream {
    pub(crate) fn new(src: &str) -> Result<Self, CliError> {
        let mut lexer = Lexer::new(src);
        let mut toks = Vec::new();
        let mut spans = Vec::new();
        loop {
            let (t, span) = lexer.next_tok()?;
            let done = t == Tok::Eof;
            toks.push(t);
            spans.push(span);
            if done {
                break;
            }
        }
        Ok(TokStream { toks, spans, pos: 0 })
    }

    pub(crate) fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    /// Peeks `offset` tokens past the current position (`0` is the same
    /// as `peek()`) without consuming anything -- clamped to the last
    /// token (`Eof`) past the stream's own end, same as `advance()`
    /// already does. Needed only for the alias-definition lookahead
    /// below (`starts_alias_def`): `:=` is two ordinary one-character
    /// `Sym` tokens, not lexed specially, so recognizing it needs two
    /// tokens of lookahead past an `Ident`, not the one `peek()` gives
    /// everywhere else in this crate.
    pub(crate) fn peek_at(&self, offset: usize) -> &Tok {
        let idx = (self.pos + offset).min(self.toks.len() - 1);
        &self.toks[idx]
    }

    /// Every name that appears anywhere in this stream as the target of
    /// an alias definition (`NOME := ...`, see `starts_alias_def`) --
    /// regardless of position. Used once, before the real top-to-bottom
    /// parse (`parse_model`), so a forward reference (`NOME` used before
    /// its own `NOME := ...` appears later in the same document) can be
    /// reported as a clean error instead of silently falling back to
    /// "just an ordinary free variable" the way a genuinely-never-declared
    /// name still does. A single linear scan over the already-lexed
    /// tokens -- no brace/structure awareness needed, since the shape
    /// being matched (`Ident`, `Sym(':')`, `Sym('=')` in a row) cannot
    /// occur anywhere else in this grammar (see `starts_alias_def`).
    pub(crate) fn scan_alias_names(&self) -> std::collections::HashSet<String> {
        let mut names = std::collections::HashSet::new();
        for i in 0..self.toks.len() {
            if let Tok::Ident(name) = &self.toks[i] {
                if self.toks.get(i + 1) == Some(&Tok::Sym(':')) && self.toks.get(i + 2) == Some(&Tok::Sym('=')) {
                    names.insert(name.clone());
                }
            }
        }
        names
    }

    /// The span of whatever `peek()` currently points at -- the position
    /// used for "found X here" in every parse error below.
    pub(crate) fn current_span(&self) -> Span {
        self.spans[self.pos]
    }

    /// Convenience for the overwhelmingly common case (a single point,
    /// not a range) -- every `CliError::Parse` this crate constructs
    /// during active parsing uses this, not a bare string.
    pub(crate) fn error(&self, message: impl Into<String>) -> CliError {
        CliError::Parse { message: message.into(), position: Some(self.current_span().start) }
    }

    pub(crate) fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    pub(crate) fn expect_sym(&mut self, c: char) -> Result<(), CliError> {
        let err = self.error(format!("expected `{c}`, found {:?}", self.peek()));
        match self.advance() {
            Tok::Sym(s) if s == c => Ok(()),
            _ => Err(err),
        }
    }

    pub(crate) fn expect_ident(&mut self, kw: &str) -> Result<(), CliError> {
        let err = self.error(format!("expected `{kw}`, found {:?}", self.peek()));
        match self.advance() {
            Tok::Ident(s) if s == kw => Ok(()),
            _ => Err(err),
        }
    }

    pub(crate) fn ident(&mut self) -> Result<String, CliError> {
        let err = self.error(format!("expected an identifier, found {:?}", self.peek()));
        match self.advance() {
            Tok::Ident(s) => Ok(s),
            _ => Err(err),
        }
    }

    pub(crate) fn int(&mut self) -> Result<u64, CliError> {
        let err = self.error(format!("expected a number, found {:?}", self.peek()));
        match self.advance() {
            Tok::Int(n) => Ok(n),
            _ => Err(err),
        }
    }

    /// Consumes a specific `\name` command token.
    pub(crate) fn expect_command(&mut self, name: &str) -> Result<(), CliError> {
        let err = self.error(format!("expected `\\{name}`, found {:?}", self.peek()));
        match self.advance() {
            Tok::Command(s) if s == name => Ok(()),
            _ => Err(err),
        }
    }
}

// ---------------------------------------------------------------------
// Worksheet entries: a second grammar production over the SAME lexer/
// TokStream/error machinery as the `.od` document grammar below --
// never a second parser (DESIGN-UI-SESSION.md, session correction: "a
// entrada da planilha tem que passar pelo MESMO PARSER do .od"). v1
// recognizes exactly one production, a bare command name optionally
// followed by which declared metric/connection to run it against
// (same disambiguation `--metric`/`resolve_choice` already does for the
// CLI, now inside the grammar instead of a flag) -- growing to
// expressions/contractions/substitutions later is a new `Query` variant
// and a new branch in `parse_query`, not a new parser.
// ---------------------------------------------------------------------

/// v1's recognized worksheet commands -- the same five `oderom-cli`
/// subcommands, now also reachable as a `Query::Command`, plus
/// `Geodesic` (Marco 6 step 4, round B) and `Accel` (round C), whose own
/// query shapes (`Query::Geodesic`/`Query::Accel`) carry an extra
/// required field none of the other five have -- see those variants'
/// own doc comments. `Accel` is spelled without an underscore
/// deliberately: `_` is not a valid identifier character in this
/// lexer at all (reserved for the LaTeX-style subscript operator,
/// `g_{tt}`), so a name like `geodesic_solved` would never lex as one
/// token in the first place -- `accel` was chosen over that shape for
/// exactly this reason, not just brevity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandName {
    Christoffel,
    Riemann,
    Ricci,
    Scalar,
    Kretschmann,
    Geodesic,
    Accel,
    /// The Einstein tensor `G_ab = R_ab - (1/2) g_ab R` (Marco 6 step
    /// 5) -- ordinary `Query::Command` shape, same as the first five:
    /// an optional target, nothing else, unlike `Geodesic`/`Accel`'s
    /// mandatory affine parameter. Needs a real metric (`g_ab` appears
    /// literally in the formula, and `R` needs `g^bd` to contract) --
    /// refuses a bare connection the same way `Scalar`/`Kretschmann`
    /// already do.
    Einstein,
    /// `R_ab R^ab` (Marco 6 step 6) -- a scalar invariant, same
    /// `Query::Command` shape as `scalar`/`kretschmann`; needs a real
    /// metric to raise both indices. Spelled without an underscore for
    /// the same lexer reason `Accel` is (`_` is the LaTeX-style
    /// subscript operator, not a valid identifier character) --
    /// `ricci_squared` would never lex as one token.
    RicciSquared,
    /// `R_abcd R^abcd - 4 R_ab R^ab + R^2` (Marco 6 step 6, the
    /// Gauss-Bonnet/Euler density) -- pure recombination of `kretschmann`/
    /// `RicciSquared`/`scalar`, same `Query::Command` shape, needs a
    /// real metric (inherited from the three scalars it combines).
    GaussBonnet,
    /// The Weyl (conformal) tensor `C_abcd`, fully covariant (Marco 6
    /// step 6) -- a tensor-listing query like `riemann`/`ricci`, not a
    /// bare scalar. Needs a real metric (`g_ab`/`R` appear literally in
    /// the formula) -- refuses a bare connection, unlike
    /// `christoffel`/`riemann`/`ricci`. Also refuses in exactly 2
    /// dimensions (the formula's own `1/(n-2)` term is undefined there)
    /// -- see `oderom_components::ComponentError::WeylUndefinedInDimension2`.
    Weyl,
    /// `C_abcd C^abcd` (Marco 6 step 6) -- a scalar invariant like
    /// `RicciSquared`, inheriting `Weyl`'s own dimension-2 refusal.
    WeylSquared,
}

/// The one canonical (keyword, variant) list -- `from_str` below and
/// [`CommandName::keywords`] both derive from this SAME array, so the
/// set of words this grammar recognizes as a command and the set a
/// caller can enumerate (e.g. a syntax highlighter, see that function's
/// own doc comment) can never independently drift apart the way two
/// hand-maintained lists can. Order here is also the order `keywords()`
/// yields them in and the order every "expected one of: ..." error
/// message lists them in.
const COMMAND_KEYWORDS: &[(&str, CommandName)] = &[
    ("christoffel", CommandName::Christoffel),
    ("riemann", CommandName::Riemann),
    ("ricci", CommandName::Ricci),
    ("scalar", CommandName::Scalar),
    ("kretschmann", CommandName::Kretschmann),
    ("geodesic", CommandName::Geodesic),
    ("accel", CommandName::Accel),
    ("einstein", CommandName::Einstein),
    ("riccisquare", CommandName::RicciSquared),
    ("gaussbonnet", CommandName::GaussBonnet),
    ("weyl", CommandName::Weyl),
    ("weylsquare", CommandName::WeylSquared),
];

impl CommandName {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        COMMAND_KEYWORDS.iter().find(|(word, _)| *word == s).map(|(_, cmd)| *cmd)
    }

    /// Every keyword this grammar recognizes as the start of a
    /// `Query::Command`/`Geodesic`/`Accel` production, in the same
    /// order `from_str` itself checks them -- the single source a
    /// syntax highlighter (`oderom-app/src-tauri/build.rs`, which
    /// generates `dist/oderom-mode.js` from exactly this list) reads
    /// instead of maintaining its own copy. This is the fix for a real,
    /// repeat bug: the highlighter's keyword list is a plain JS regex,
    /// disconnected from this parser, and had already gone stale twice
    /// (once for `sin`/`cosh`/`exp`-family functions, once for `export`
    /// itself) before this function existed to generate it from instead
    /// of hand-copying it.
    pub fn keywords() -> impl Iterator<Item = &'static str> {
        COMMAND_KEYWORDS.iter().map(|(word, _)| *word)
    }
}

/// The `export` keyword (Rodada Exportação) -- one canonical constant,
/// used at every site that recognizes it (`parse_query_tokens`,
/// `starts_new_top_level_statement`, `classify_block`) instead of the
/// literal `"export"` repeated at each, and read by the syntax
/// highlighter's generator alongside [`CommandName::keywords`]/
/// [`DECLARATION_KEYWORDS`] for the same reason.
pub const EXPORT_KEYWORD: &str = "export";

/// The two words an index-variance marker accepts (`riemann
/// [up,down,down,down]`, section 9.16/[`parse_variance_bracket`]) --
/// canonical constants for the same reason `EXPORT_KEYWORD` is: one
/// spelling, read by both the parser's own match and the syntax
/// highlighter's generator, never two copies that could disagree.
pub const VARIANCE_UP: &str = "up";
pub const VARIANCE_DOWN: &str = "down";

/// One worksheet entry, parsed. `Command`'s `target` is `None` unless
/// the entry named a specific declared metric/connection -- resolving
/// `None` against a `Model` uses the exact same "flag, or the sole
/// entry, or an explicit ambiguity error" rule `resolve_choice`
/// (`commands.rs`) already implements; `oderom-session` reuses that
/// function rather than a new one.
///
/// `Geodesic`/`Accel`'s `param` is the user-named affine parameter
/// (Marco 6 step 4, rounds B/C) -- always required, unlike `target`,
/// which stays optional exactly as it is for the other five (resolved
/// the same "flag, or the sole metric/connection, or an explicit
/// ambiguity error" way). Separate variants, not another `Command`
/// field, because the other five commands have no parameter at all and
/// giving them an always-`None` one would be exactly the kind of
/// "assigns a field to a case that can't use it" abstraction this
/// project avoids. `Geodesic`/`Accel` are two distinct variants rather
/// than one with a "solved: bool" flag for the same reason `Command`
/// stays separate from both: they render differently (canonical "= 0"
/// vs. solved-for-the-acceleration) and go through different
/// `oderom-components::curvature` functions -- a boolean toggling that
/// downstream behavior would just move the same two-way branch one
/// level up without actually simplifying anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Command { name: Spanned<CommandName>, target: Option<Spanned<String>>, variance: Option<Spanned<Vec<Variance>>> },
    Geodesic { target: Option<Spanned<String>>, param: Spanned<String> },
    Accel { target: Option<Spanned<String>>, param: Spanned<String> },
    /// `export TARGET QUERY` (Rodada Exportação): translate whatever
    /// `inner` produces into Mathematica or SymPy source instead of this
    /// project's own Unicode/LaTeX rendering. `format` is already
    /// resolved to one of the two symbolic [`Target`] variants this
    /// round supports -- `export`'s own target word is a fixed, closed
    /// set of names (unlike `Command`'s `target` field, which names a
    /// user-declared metric/connection and can't be validated until a
    /// `Model` exists), so an unknown word is rejected right here, at
    /// parse time, with the same span-carrying `CliError::Parse` every
    /// other grammar mistake gets -- see [`parse_export_query_tail`].
    Export { format: Spanned<Target>, inner: Box<Query> },
}

/// `QUERY := COMMAND_NAME IDENT? VARIANCE? | ('geodesic'|'accel') IDENT? IDENT` --
/// `COMMAND_NAME` one of the first ten in [`CommandName`], the optional
/// `IDENT` naming which declared metric or connection to run it against,
/// and `VARIANCE` an optional trailing index-variance marker (Rodada
/// Variancia, see [`parse_variance_bracket`]) -- `riemann g
/// [up,down,down,down]` reads target then marker, in that order, the
/// same "most specific qualifier last" order the grammar already uses
/// for `geodesic`'s own target-then-parameter (`geodesic g tau`).
/// `geodesic`/`accel` never take a variance marker at all -- they
/// produce equations, not indexed tensor objects, so there is nothing
/// for `up`/`down` to mean there; their own production is unchanged.
///
/// Whether a given `COMMAND_NAME` actually *accepts* a marker (barred
/// for Christoffel, meaningless for a bare scalar) and whether its
/// length matches the named tensor's rank are NOT checked here -- same
/// "grammar checks shape, the caller checks meaning" split this
/// production already uses for `target` (a name that doesn't resolve to
/// any declared metric/connection is also not a parse error). See
/// `oderom-session::run::run_command_query`'s own variance-validation
/// step for where that happens.
///
/// Consumes the whole stream: any token left over after that is a parse
/// error, not silently ignored trailing input. Same shape as
/// [`parse_model`]/[`parse_monomial`]: takes source text directly, not a
/// `TokStream` (kept crate-private -- an implementation detail of *how*
/// parsing happens, never part of this crate's public surface).
pub fn parse_query(src: &str) -> Result<Query, CliError> {
    let mut toks = TokStream::new(src)?;
    parse_query_tokens(&mut toks)
}

fn parse_query_tokens(toks: &mut TokStream) -> Result<Query, CliError> {
    let name_span = toks.current_span();
    let name_str = toks.ident()?;

    if name_str == EXPORT_KEYWORD {
        return parse_export_query_tail(toks);
    }

    let Some(name) = CommandName::from_str(&name_str) else {
        return Err(CliError::Parse {
            message: format!(
                "unknown command `{name_str}` (expected one of: christoffel, riemann, ricci, scalar, kretschmann, geodesic, accel, einstein, riccisquare, gaussbonnet, weyl, weylsquare, export)"
            ),
            position: Some(name_span.start),
        });
    };

    if name == CommandName::Geodesic || name == CommandName::Accel {
        return parse_geodesic_like_query_tail(toks, name);
    }

    let name = Spanned::new(name, name_span);

    // A target is a bare identifier; anything else (including `[`, the
    // start of a variance marker, or `Eof`) means there is none. Every
    // token that isn't `Ident`/`Sym('[')`/`Eof` here was already a hard
    // parse error before this round (this production only ever accepted
    // an identifier or nothing) -- `[` is the one genuinely new
    // continuation this grammar recognizes.
    let target = match toks.peek() {
        Tok::Ident(_) => {
            let target_span = toks.current_span();
            let target_name = toks.ident()?;
            Some(Spanned::new(target_name, target_span))
        }
        _ => None,
    };

    let variance = if *toks.peek() == Tok::Sym('[') { Some(parse_variance_bracket(toks)?) } else { None };

    if *toks.peek() != Tok::Eof {
        return Err(toks.error(format!("unexpected extra input after the command, found {:?}", toks.peek())));
    }

    Ok(Query::Command { name, target, variance })
}

/// `'[' ('up'|'down') (',' ('up'|'down'))* ']'` -- the index-variance
/// marker (Rodada Variancia): one `up`/`down` per slot of the tensor the
/// query names, `up` meaning "this index should come out contravariant
/// (raised with `g^ab` if it isn't already)", `down` meaning "covariant
/// (lowered with `g_ab` if it isn't already)". Maps directly onto
/// [`Variance::Contra`]/[`Variance::Co`] rather than a separate
/// up/down-specific enum -- it IS the same concept
/// [`oderom_components::curvature::change_variance`] and the renderer's
/// own `format_indices` already use, so there is no translation step
/// anywhere downstream, only at this one parsing boundary (English
/// words in the surface syntax, the crate's own variance type
/// internally).
///
/// `[`/`]`/`,` were deliberately chosen reusing symbols this lexer
/// already emits as plain, uncontested `Sym` tokens: metric/connection
/// component declarations (`[i,j] = expr`) and `NAME[a,b,c,d]`
/// tensor-monomial factors both already use them, and neither
/// production can ever appear where this one starts (right after a
/// query's own command name/target -- never inside a `{ ... }`
/// component block, never inside a `canon` expression argument), so
/// there is no grammar ambiguity to resolve and no new lexer work at
/// all.
fn parse_variance_bracket(toks: &mut TokStream) -> Result<Spanned<Vec<Variance>>, CliError> {
    let open_span = toks.current_span();
    toks.expect_sym('[')?;
    let mut items = Vec::new();
    loop {
        let word_span = toks.current_span();
        let word = toks.ident()?;
        let v = match word.as_str() {
            VARIANCE_UP => Variance::Contra,
            VARIANCE_DOWN => Variance::Co,
            other => {
                return Err(CliError::Parse {
                    message: format!("expected `up` or `down` in an index-variance marker, found `{other}`"),
                    position: Some(word_span.start),
                })
            }
        };
        items.push(v);
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    let close_span = toks.current_span();
    toks.expect_sym(']')?;
    Ok(Spanned::new(items, Span { start: open_span.start, end: close_span.end }))
}

/// `geodesic`/`accel`'s shared tail, after the keyword itself is
/// already consumed: one trailing identifier is the parameter alone,
/// two are target then parameter -- see [`parse_query`]'s own doc
/// comment for the grammar and reasoning. `name` is which of the two
/// keywords was actually seen, so the one, shared parsing routine below
/// still builds the right `Query` variant and names the right command
/// in its own error message.
fn parse_geodesic_like_query_tail(toks: &mut TokStream, name: CommandName) -> Result<Query, CliError> {
    let command_word = match name {
        CommandName::Geodesic => "geodesic",
        CommandName::Accel => "accel",
        _ => unreachable!("only ever called for Geodesic or Accel"),
    };

    if *toks.peek() == Tok::Eof {
        return Err(toks.error(format!("`{command_word}` needs an affine parameter name, e.g. `{command_word} tau` or `{command_word} g tau`")));
    }
    let first_span = toks.current_span();
    let first = toks.ident()?;

    let (target, param) = if *toks.peek() == Tok::Eof {
        (None, Spanned::new(first, first_span))
    } else {
        let param_span = toks.current_span();
        let param_name = toks.ident()?;
        (Some(Spanned::new(first, first_span)), Spanned::new(param_name, param_span))
    };

    if *toks.peek() != Tok::Eof {
        return Err(toks.error(format!("unexpected extra input after the command, found {:?}", toks.peek())));
    }

    Ok(match name {
        CommandName::Geodesic => Query::Geodesic { target, param },
        CommandName::Accel => Query::Accel { target, param },
        _ => unreachable!("only ever called for Geodesic or Accel"),
    })
}

/// `'export' ('mathematica'|'sympy') QUERY` -- `export`'s own tail,
/// after the keyword itself is already consumed (Rodada Exportação).
/// Validates the target word against exactly the two symbolic targets
/// this round supports right here (a fixed, closed set, unlike a
/// metric/connection `target` name, which can only be checked once a
/// `Model` exists -- see `Query::Export`'s own doc comment), then
/// recurses into [`parse_query_tokens`] itself for the inner query --
/// `export sympy riemann g [up,down,down,down]` is `export` followed by
/// exactly the same `QUERY` production this grammar already has, not a
/// second, parallel one. The recursive call checks `Eof` at its own
/// end, so trailing garbage after the inner query (`export sympy
/// kretschmann extra`) is caught there, not by a second check here.
/// The one canonical (keyword, `Target`) list for `export`'s own
/// target word -- same "one array, two consumers" shape as
/// `COMMAND_KEYWORDS`/[`CommandName::keywords`]: `parse_export_query_tail`
/// below and [`export_target_keywords`] both read this, so the parser's
/// accepted spellings and the syntax highlighter's keyword list can
/// never disagree.
const EXPORT_TARGET_KEYWORDS: &[(&str, Target)] = &[("mathematica", Target::Mathematica), ("sympy", Target::Sympy)];

/// Every word `export` accepts as its target, in the same order
/// `parse_export_query_tail` itself checks them -- read by the syntax
/// highlighter's generator (`oderom-app/src-tauri/build.rs`), same
/// reason as [`CommandName::keywords`].
pub fn export_target_keywords() -> impl Iterator<Item = &'static str> {
    EXPORT_TARGET_KEYWORDS.iter().map(|(word, _)| *word)
}

fn parse_export_query_tail(toks: &mut TokStream) -> Result<Query, CliError> {
    if *toks.peek() == Tok::Eof {
        return Err(toks.error("`export` needs a target and a query, e.g. `export sympy kretschmann` or `export mathematica riemann g`"));
    }
    let target_span = toks.current_span();
    let target_str = toks.ident()?;
    let Some((_, format)) = EXPORT_TARGET_KEYWORDS.iter().find(|(word, _)| *word == target_str) else {
        return Err(CliError::Parse {
            message: format!("unknown export target `{target_str}` (expected one of: mathematica, sympy)"),
            position: Some(target_span.start),
        });
    };
    let format = *format;

    if *toks.peek() == Tok::Eof {
        return Err(toks.error(format!("`export {target_str}` needs a query to export, e.g. `export {target_str} kretschmann`")));
    }
    let inner_span = toks.current_span();
    let inner = parse_query_tokens(toks)?;
    // `export sympy export mathematica kretschmann` -- nesting export
    // inside export has no meaning (there is nothing left to translate
    // a second time) and is rejected here, at parse time, rather than
    // left to whatever `run_query_to_rendered` would do with it
    // (`oderom-session::run` treats a nested `Query::Export` as
    // unreachable precisely because this check rules it out earlier).
    if matches!(inner, Query::Export { .. }) {
        return Err(CliError::Parse { message: "`export` cannot be nested inside another `export`".to_string(), position: Some(inner_span.start) });
    }
    Ok(Query::Export { format: Spanned::new(format, target_span), inner: Box::new(inner) })
}

/// `oderom-notebook`'s "no separate mode, the parser decides" design
/// (DESIGN-NOTEBOOK.md section 1) rests on this: the six declaration
/// keywords `parse_model`'s own loop recognizes below
/// (`manifold`/`bundle`/`head`/`chart`/`metric`/`connection`) and the
/// twelve query keywords `CommandName::from_str` recognizes are disjoint
/// sets, so peeking a block's very first token is enough to route it to
/// `parse_model` or `parse_query` without ambiguity -- not a second
/// grammar, just a named entry point for a distinction the lexer
/// already makes (`parse_model`'s own doc comment: "statements are
/// recognized by their leading keyword").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Declaration,
    Query,
}

/// `pub`, not `pub(crate)`: read directly by `oderom-app/src-tauri/build.rs`
/// (via `oderom-cli` as a build-dependency) to generate the syntax
/// highlighter's keyword list -- see [`CommandName::keywords`]'s own
/// doc comment for why this project now generates that list instead of
/// hand-maintaining a second copy of it.
pub const DECLARATION_KEYWORDS: [&str; 6] = ["manifold", "bundle", "head", "chart", "metric", "connection"];

/// Whether the token at `toks`'s CURRENT position (assumed to already be
/// an `Ident`) begins an alias definition `NOME := EXPR` -- two tokens
/// of lookahead, no consumption. Safe to recognize this way (never a
/// false positive against the rest of the grammar): `:=` is two
/// one-character `Sym` tokens, `Sym(':')` immediately followed by
/// `Sym('=')`, and the only other place a bare `:` appears at all is
/// `head NAME : SLOT` (`parse_head_decl`), where it is never immediately
/// followed by `=`.
pub(crate) fn starts_alias_def(toks: &TokStream) -> bool {
    *toks.peek_at(1) == Tok::Sym(':') && *toks.peek_at(2) == Tok::Sym('=')
}

/// Whether `name` -- the identifier `toks` is CURRENTLY positioned at --
/// should be read as the start of a brand new top-level statement rather
/// than a continuation, by juxtaposition, of whatever `SCALAR_EXPR` is
/// currently being parsed. Every OTHER `SCALAR_EXPR` use site is
/// unambiguously delimited by `,`, `}`, or end of input (a metric/
/// connection component line, a query's target) -- an alias definition
/// (`NOME := EXPR`) is the one place this grammar has an expression with
/// NO closing token at all, so without this check, `parse_product`'s
/// ordinary juxtaposition rule (`expr_parser.rs`'s `starts_atom`, which
/// treats any following identifier as an implicit multiplication factor
/// -- `2M` needs this) would just keep eating tokens across the boundary
/// into whatever statement comes next: `f := 1 - 2*M/r\nchart ...` would
/// silently read as `(1 - 2*M/r) * chart`, and `f := 1 - 2*M/r\ng :=
/// f^2` would silently read `f`'s own value as `(1 - 2*M/r) * g` --
/// found exactly this way, by a real test, not anticipated in the
/// abstract. Two cases stop it: `name` is one of the eleven reserved
/// keywords (six declaration, five query -- `2 metric` was never
/// meaningful as "multiply by a variable named metric" to begin with, so
/// nothing legitimate is lost; an explicit `2*metric` still works,
/// juxtaposition is the only thing withheld), or `name` is itself about
/// to become ANOTHER alias definition (`starts_alias_def`).
pub(crate) fn starts_new_top_level_statement(toks: &TokStream, name: &str) -> bool {
    DECLARATION_KEYWORDS.contains(&name) || CommandName::from_str(name).is_some() || name == EXPORT_KEYWORD || starts_alias_def(toks)
}

/// Classifies `src` by its leading keyword alone -- does not otherwise
/// validate it (a block that classifies as `Declaration` here can still
/// fail `parse_model`, same as any `.od` text always could). `Err` only
/// when the first token isn't a recognized keyword, a recognized query
/// name, or the start of an alias definition (including an empty block,
/// whose first token is `Eof`) -- a third, explicit case a notebook
/// block can show, never a silent guess between the other two.
///
/// An alias definition classifies as `Declaration` (never a fourth
/// `BlockKind`): it *is* one, in every way this enum's two consumers
/// care about -- `reconstruct_declarations` (`oderom-notebook`) needs to
/// concatenate it together with every other declaration block and
/// reevaluate as one unit (exactly what gives an alias its "declared
/// before used, notebook-wide from that point down" scope, via the
/// exact same mechanism `chart`/`metric`/... already use -- no new
/// notebook-level machinery), and editing it must cascade-obsolete
/// blocks below it exactly like editing a `chart` would. See
/// `DESIGN-NOTEBOOK.md`'s alias section for the reasoning in full.
pub fn classify_block(src: &str) -> Result<BlockKind, CliError> {
    let toks = TokStream::new(src)?;
    match toks.peek() {
        Tok::Ident(kw) if DECLARATION_KEYWORDS.contains(&kw.as_str()) => Ok(BlockKind::Declaration),
        Tok::Ident(kw) if kw == EXPORT_KEYWORD || CommandName::from_str(kw).is_some() => Ok(BlockKind::Query),
        Tok::Ident(_) if starts_alias_def(&toks) => Ok(BlockKind::Declaration),
        other => Err(CliError::Parse {
            message: format!(
                "expected a declaration (manifold/bundle/head/chart/metric/connection), an alias (NOME := expression), or a query (christoffel/riemann/ricci/scalar/kretschmann/geodesic/accel/einstein/riccisquare/gaussbonnet/weyl/weylsquare/export), found {other:?}"
            ),
            position: Some(toks.current_span().start),
        }),
    }
}

// ---------------------------------------------------------------------
// prelude.od: `manifold`, `bundle`, `head` declarations
// ---------------------------------------------------------------------

/// Parses a `.od` source into a populated [`Model`] -- one language for
/// everything (DESIGN-UI.md 6.1): Marco 1's abstract declarations
/// (`manifold`/`bundle`/`head`) and the UI CLI's concrete ones
/// (`chart`/`metric`/`connection`) share the same statement loop, the
/// same lexer, and (for `metric`/`connection` component values) the same
/// `SCALAR_EXPR` grammar with its ASCII/LaTeX token synonyms.
///
/// Grammar (statements are recognized by their leading keyword; no
/// terminator is needed since every construct's own grammar determines
/// where it ends):
///
/// ```text
/// manifold NAME dim N
/// bundle NAME on MANIFOLD dim N
/// head NAME : SLOT (, SLOT)* [symmetry GEN+]
/// SLOT      := BUNDLE ['*']            (bare = contravariant, * = covariant)
/// GEN       := 'antisymmetric' | 'symmetric' | CYCLE+ ('+' | '-')
/// CYCLE     := '(' INT+ ')'
///
/// chart NAME on MANIFOLD coords (IDENT (',' IDENT)*)
/// metric NAME on CHART bundle BUNDLE '{' LINE (',' LINE)* '}'
/// connection NAME on CHART '{' LINE (',' LINE)* '}'
/// LINE := '[' IDX (',' IDX)* ']' '=' SCALAR_EXPR
///       | NAME '_' '{' ... '}' '=' SCALAR_EXPR    (NAME echoes the decl's own name)
/// IDX  := coordinate name of the referenced CHART, or a literal integer
/// ```
///
/// `metric` always declares its head as symmetric rank 2 (a metric *is*
/// that, not user data); `connection` has no symmetry group at all
/// (Christoffel symbols aren't a tensor, same as `curvature::christoffel`
/// already assumes).
pub fn parse_model(src: &str) -> Result<Model, CliError> {
    let mut toks = TokStream::new(src)?;
    // Computed once, up front, over the WHOLE document -- every alias
    // name this text will ever declare, regardless of position. Used
    // only to tell "genuinely never an alias" (silently an ordinary free
    // variable, unchanged from before this feature existed) apart from
    // "an alias, but not declared yet at this point" (a forward
    // reference, a clean error) -- see `AliasScope`'s own doc comment.
    let alias_names = toks.scan_alias_names();
    let mut model = Model::new();

    loop {
        match toks.peek().clone() {
            Tok::Eof => break,
            Tok::Ident(kw) if kw == "manifold" => {
                toks.advance();
                let name = toks.ident()?;
                toks.expect_ident("dim")?;
                let dim = toks.int()? as u32;
                model.registry.declare_manifold(&name, dim)?;
            }
            Tok::Ident(kw) if kw == "bundle" => {
                toks.advance();
                let name = toks.ident()?;
                toks.expect_ident("on")?;
                let manifold_name = toks.ident()?;
                let manifold = model.registry.lookup_manifold(&manifold_name)?;
                toks.expect_ident("dim")?;
                let dim = toks.int()? as u32;
                model.registry.declare_bundle(&name, manifold, dim)?;
            }
            Tok::Ident(kw) if kw == "head" => {
                toks.advance();
                parse_head_decl(&mut toks, &mut model.registry)?;
            }
            Tok::Ident(kw) if kw == "chart" => {
                toks.advance();
                parse_chart_decl(&mut toks, &mut model)?;
            }
            Tok::Ident(kw) if kw == "metric" => {
                toks.advance();
                parse_metric_decl(&mut toks, &mut model, &alias_names)?;
            }
            Tok::Ident(kw) if kw == "connection" => {
                toks.advance();
                parse_connection_decl(&mut toks, &mut model, &alias_names)?;
            }
            // Checked last among the `Ident` arms, same reasoning as
            // `classify_block`'s own ordering: a name that collides with
            // one of the six keywords above (or, further down at
            // statement level, is never reached here at all since
            // queries have their own separate grammar) is still caught
            // by ITS OWN arm first, so e.g. `chart := 5` fails inside
            // `parse_chart_decl` with a real (if not maximally specific)
            // error rather than silently becoming an alias -- consistent
            // with this project's "clear error over silent guess" rule,
            // not a gap.
            Tok::Ident(name) if starts_alias_def(&toks) => {
                toks.advance();
                toks.expect_sym(':')?;
                toks.expect_sym('=')?;
                parse_alias_decl(&mut toks, &mut model, name, &alias_names)?;
            }
            other => return Err(toks.error(format!("expected a declaration or an alias (NOME := expression), found {other:?}"))),
        }
    }

    Ok(model)
}

/// `NOME := EXPR` (`starts_alias_def`/`classify_block` recognize the
/// leading shape; this does the actual parse once `parse_model`'s own
/// loop has already consumed `NOME :=`). Pure syntactic sugar: `expr` is
/// parsed against `model.aliases` AS BUILT SO FAR (never the final,
/// whole-document map) and stored fully expanded -- an alias's own
/// right-hand side can reference an earlier alias (also already
/// expanded by the time it's stored, so expansion never has to recurse
/// at lookup time), but never itself or one declared later (that is
/// exactly the forward-reference case `AliasScope`/`alias_names`
/// catches, with a clean error, not silent misresolution as a bare
/// variable). Redefining the same name is rejected the same way
/// `chart`'s own duplicate check already is -- checked after parsing the
/// right-hand side (matching `parse_chart_decl`'s own ordering: a
/// duplicate whose own right-hand side also fails to parse reports THAT
/// error, not a swallowed one).
fn parse_alias_decl(toks: &mut TokStream, model: &mut Model, name: String, alias_names: &std::collections::HashSet<String>) -> Result<(), CliError> {
    let scope = crate::expr_parser::AliasScope { declared: &model.aliases, all_names: alias_names };
    let expr = crate::expr_parser::parse_scalar_expr(toks, &scope)?;
    if model.aliases.contains_key(&name) {
        return Err(toks.error(format!("alias `{name}` is already declared")));
    }
    model.aliases.insert(name, expr);
    Ok(())
}

fn parse_chart_decl(toks: &mut TokStream, model: &mut Model) -> Result<(), CliError> {
    let name = toks.ident()?;
    toks.expect_ident("on")?;
    let manifold_name = toks.ident()?;
    let manifold = model.registry.lookup_manifold(&manifold_name)?;
    toks.expect_ident("coords")?;
    toks.expect_sym('(')?;
    let mut coords = Vec::new();
    loop {
        coords.push(toks.ident()?);
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    toks.expect_sym(')')?;

    let dim = model.registry.manifold(manifold).dim as usize;
    if coords.len() != dim {
        return Err(toks.error(format!(
            "chart `{name}` has {} coordinates, but manifold `{manifold_name}` has dimension {dim}",
            coords.len()
        )));
    }
    if model.charts.contains_key(&name) {
        return Err(toks.error(format!("chart `{name}` is already declared")));
    }
    model.charts.insert(name, Chart::new(coords));
    Ok(())
}

fn lookup_chart<'a>(model: &'a Model, toks: &TokStream, name: &str) -> Result<&'a Chart, CliError> {
    model.charts.get(name).ok_or_else(|| toks.error(format!("unknown chart `{name}`")))
}

fn parse_metric_decl(toks: &mut TokStream, model: &mut Model, alias_names: &std::collections::HashSet<String>) -> Result<(), CliError> {
    let name = toks.ident()?;
    toks.expect_ident("on")?;
    let chart_name = toks.ident()?;
    let chart = lookup_chart(model, toks, &chart_name)?.clone();
    toks.expect_ident("bundle")?;
    let bundle_name = toks.ident()?;
    let bundle = model.registry.lookup_bundle(&bundle_name)?;
    let dim = model.registry.bundle(bundle).dim;
    let co = SlotSig { bundle, variance: Variance::Co, dim };
    let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let head = model.registry.declare_head(&name, slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])?;

    let mut tensor = ComponentTensor::new(head);
    toks.expect_sym('{')?;
    loop {
        let scope = crate::expr_parser::AliasScope { declared: &model.aliases, all_names: alias_names };
        let (indices, expr) = parse_component_line(toks, &chart, 2, &name, &scope)?;
        tensor.set_declared(&model.registry, &indices, normalize(&expr))?;
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    toks.expect_sym('}')?;

    model.metrics.insert(name, (chart_name, head, tensor));
    Ok(())
}

fn parse_connection_decl(toks: &mut TokStream, model: &mut Model, alias_names: &std::collections::HashSet<String>) -> Result<(), CliError> {
    let name = toks.ident()?;
    toks.expect_ident("on")?;
    let chart_name = toks.ident()?;
    let chart = lookup_chart(model, toks, &chart_name)?.clone();

    let mut gamma = Grid::new(chart.dim(), 3);
    toks.expect_sym('{')?;
    loop {
        let scope = crate::expr_parser::AliasScope { declared: &model.aliases, all_names: alias_names };
        let (indices, expr) = parse_component_line(toks, &chart, 3, &name, &scope)?;
        gamma.set(&indices, normalize(&expr));
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    toks.expect_sym('}')?;

    model.connections.insert(name, (chart_name, gamma));
    Ok(())
}

fn parse_head_decl(toks: &mut TokStream, reg: &mut Registry) -> Result<(), CliError> {
    let name = toks.ident()?;
    toks.expect_sym(':')?;

    let mut slots: SmallVec<[SlotSig; 4]> = SmallVec::new();
    loop {
        let bundle_name = toks.ident()?;
        let variance = if *toks.peek() == Tok::Sym('*') {
            toks.advance();
            Variance::Co
        } else {
            Variance::Contra
        };
        let bundle = reg.lookup_bundle(&bundle_name)?;
        let dim = reg.bundle(bundle).dim;
        slots.push(SlotSig { bundle, variance, dim });

        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }

    let arity = slots.len();
    let mut generators: Vec<SignedPerm> = Vec::new();
    if let Tok::Ident(kw) = toks.peek() {
        if kw == "symmetry" {
            toks.advance();
            generators = parse_generators(toks, arity)?;
        }
    }

    reg.declare_head(&name, slots, generators)?;
    Ok(())
}

fn parse_generators(toks: &mut TokStream, arity: usize) -> Result<Vec<SignedPerm>, CliError> {
    let mut gens = Vec::new();
    loop {
        match toks.peek().clone() {
            Tok::Ident(kw) if kw == "antisymmetric" => {
                toks.advance();
                gens.extend(oderom_core::totally_antisymmetric_generators(arity));
            }
            Tok::Ident(kw) if kw == "symmetric" => {
                toks.advance();
                gens.extend(totally_symmetric_generators(arity));
            }
            Tok::Sym('(') => {
                let mut images: Vec<u16> = (0..arity as u16).collect();
                loop {
                    toks.expect_sym('(')?;
                    let mut cycle = Vec::new();
                    loop {
                        cycle.push(toks.int()? as u16);
                        if *toks.peek() == Tok::Sym(')') {
                            break;
                        }
                    }
                    toks.expect_sym(')')?;
                    apply_cycle(toks, &mut images, &cycle, arity)?;
                    if *toks.peek() != Tok::Sym('(') {
                        break;
                    }
                }
                let sign_err = toks.error(format!(
                    "expected `+` or `-` after a symmetry generator's cycles, found {:?}",
                    toks.peek()
                ));
                let sign = match toks.advance() {
                    Tok::Sym('+') => 1,
                    Tok::Sym('-') => -1,
                    _ => return Err(sign_err),
                };
                gens.push(SignedPerm::new(Perm::from_images(images), sign));
            }
            _ => break,
        }
    }
    Ok(gens)
}

fn apply_cycle(toks: &TokStream, images: &mut [u16], cycle: &[u16], arity: usize) -> Result<(), CliError> {
    for &c in cycle {
        if c == 0 || c as usize > arity {
            return Err(toks.error(format!(
                "symmetry generator references slot {c}, but the head has arity {arity} (slots are 1-based)"
            )));
        }
    }
    for i in 0..cycle.len() {
        let from = cycle[i] as usize - 1;
        let to = cycle[(i + 1) % cycle.len()] as usize - 1;
        images[from] = to as u16;
    }
    Ok(())
}

fn totally_symmetric_generators(n: usize) -> Vec<SignedPerm> {
    (0..n.saturating_sub(1))
        .map(|i| SignedPerm::new(Perm::transposition(n, i as u16, i as u16 + 1), 1))
        .collect()
}

// ---------------------------------------------------------------------
// Expression: a single monomial, e.g. `-3/4 R[a,b,c,d] g[a,c]`
// ---------------------------------------------------------------------

struct ParsedFactor {
    head_name: String,
    indices: Vec<String>,
    /// How many of `indices`'s trailing entries came after a `;`
    /// -- covariant-derivative indices, in the standard GR spelling
    /// (`T[a,b;c]` is the tensor `nabla_c T_ab`).
    derivatives: u8,
    /// Symmetrisation groups, as position ranges into `indices`.
    sym_groups: Vec<SymGroup>,
}

/// A run of index positions to be (anti)symmetrised over: `T[(a,b)]` is
/// `(T[a,b] + T[b,a])/2`, `T[[a,b]]` is `(T[a,b] - T[b,a])/2`.
///
/// A group may straddle the `;`. That is not an oversight to be
/// forbidden: `R_{ab[cd;e]}` -- the second Bianchi identity's own
/// spelling -- antisymmetrises two base indices together with a
/// derivative index. Permuting labels never changes *which* positions
/// are derivative slots, so the straddling case needs no special care.
#[derive(Clone, Copy)]
struct SymGroup {
    start: usize,
    len: usize,
    antisym: bool,
}

/// Parses and resolves a single tensor monomial against `registry`:
/// coefficient, factors by juxtaposition, and index names in brackets.
/// An index name appearing exactly twice across the whole monomial
/// becomes a contraction; exactly once, a free index; any other count is
/// a parse error.
pub fn parse_monomial(src: &str, registry: &mut Registry) -> Result<Monomial, CliError> {
    let mut expanded = parse_monomial_expanded(src, registry)?;
    if expanded.len() != 1 {
        return Err(CliError::Parse {
            message: format!(
                "simetrizacao abrevia uma soma de {} monomios, e aqui e esperado um monomio unico; use `oderom simplify`",
                expanded.len()
            ),
            position: None,
        });
    }
    Ok(expanded.pop().expect("length checked to be 1 just above"))
}

/// `parse_monomial`'s real body. Returns a *vector* because a
/// symmetrisation group abbreviates a sum: `T[(a,b)]` is two monomials,
/// not one. Callers that genuinely need a single monomial (`canon`, a
/// Marco 1 operation on one term) go through `parse_monomial` above and
/// get a clear error instead of a silently dropped term.
fn parse_monomial_expanded(src: &str, registry: &mut Registry) -> Result<Vec<Monomial>, CliError> {
    let mut toks = TokStream::new(src)?;

    let mut coeff_sign = 1i64;
    if *toks.peek() == Tok::Sym('-') {
        toks.advance();
        coeff_sign = -1;
    } else if *toks.peek() == Tok::Sym('+') {
        toks.advance();
    }

    let saw_coefficient = matches!(toks.peek(), Tok::Int(_));
    let coeff = if let Tok::Int(_) = toks.peek() {
        let num = toks.int()? as i64;
        if *toks.peek() == Tok::Sym('/') {
            toks.advance();
            let den = toks.int()? as i64;
            Scalar::new(coeff_sign * num, den)
        } else {
            Scalar::new(coeff_sign * num, 1)
        }
    } else {
        Scalar::new(coeff_sign, 1)
    };

    let mut factors = Vec::new();
    loop {
        match toks.peek().clone() {
            Tok::Ident(_) => factors.push(parse_factor(&mut toks)?),
            Tok::Eof => break,
            other => return Err(toks.error(format!("expected a tensor factor, found {other:?}"))),
        }
    }
    if factors.is_empty() && !saw_coefficient {
        return Err(toks.error("expected a rational coefficient, at least one tensor factor, or both"));
    }
    // A bare coefficient with no factors is a legitimate monomial -- a
    // scalar. It is not a curiosity of the grammar: it is what this
    // project's own tools *print*. `g[a,a]` with the metric eliminated
    // is the dimension (`4`), and a sum that cancels is `0`, which is
    // the single most common thing `simplify` outputs. Rejecting it on
    // input meant a reader could not paste a result back in to keep
    // working -- the renderer could emit text its own parser refused.

    expand_symmetrisations(coeff, &factors, &toks)?
        .into_iter()
        .map(|(c, labels)| {
            let rewritten: Vec<ParsedFactor> = factors
                .iter()
                .zip(labels)
                .map(|(f, indices)| ParsedFactor {
                    head_name: f.head_name.clone(),
                    indices,
                    derivatives: f.derivatives,
                    sym_groups: Vec::new(),
                })
                .collect();
            build_monomial(c, rewritten, registry)
        })
        .collect()
}

fn parse_factor(toks: &mut TokStream) -> Result<ParsedFactor, CliError> {
    let head_name = toks.ident()?;
    toks.expect_sym('[')?;
    let mut indices = Vec::new();
    // Everything after the first `;` is a covariant-derivative index --
    // the standard GR spelling (comma for partial, semicolon for
    // covariant), so `T[a,b;c]` reads as `nabla_c T_ab`. Kept inside the
    // existing `HEAD[...]` bracket grammar rather than introducing a
    // prefix operator, so nothing else about a factor changes.
    let mut derivatives: u8 = 0;
    let mut seen_semicolon = false;
    let mut sym_groups: Vec<SymGroup> = Vec::new();
    // `(antisym, start)` of the group currently open, if any. Only one
    // may be open at a time: nesting is rejected rather than guessed at.
    let mut open: Option<(bool, usize)> = None;
    loop {
        // A loop, not a single check: consecutive openers (`[((a,b),c)]`)
        // must be caught here as nesting. Testing only once per iteration
        // would let the second `(` fall through to `ident()` and be
        // reported as "expected an identifier", which names the symptom
        // instead of what the user wrote.
        while let Tok::Sym(c @ ('(' | '[')) = toks.peek() {
            let antisym = *c == '[';
            if open.is_some() {
                return Err(toks.error(
                    "grupos de simetrizacao nao podem ser aninhados; feche o grupo aberto antes de abrir outro",
                ));
            }
            toks.advance();
            open = Some((antisym, indices.len()));
        }
        indices.push(toks.ident()?);
        if seen_semicolon {
            derivatives += 1;
        }
        // A `]` closes an open antisymmetrisation group; otherwise it is
        // the factor's own closing bracket, left for `expect_sym` below.
        match (toks.peek(), open) {
            (Tok::Sym(')'), Some((false, start))) => {
                toks.advance();
                sym_groups.push(SymGroup { start, len: indices.len() - start, antisym: false });
                open = None;
            }
            (Tok::Sym(']'), Some((true, start))) => {
                toks.advance();
                sym_groups.push(SymGroup { start, len: indices.len() - start, antisym: true });
                open = None;
            }
            (Tok::Sym(')'), _) => {
                return Err(toks.error("`)` sem `(` correspondente na lista de indices"));
            }
            _ => {}
        }
        match toks.peek() {
            Tok::Sym(',') => {
                toks.advance();
            }
            Tok::Sym(';') => {
                if seen_semicolon {
                    return Err(toks.error("only one `;` may appear in an index list; write every derivative index after it (`T[a,b;c,d]`)"));
                }
                seen_semicolon = true;
                toks.advance();
            }
            _ => break,
        };
    }
    if seen_semicolon && derivatives == 0 {
        return Err(toks.error("`;` must be followed by at least one derivative index"));
    }
    if let Some((antisym, _)) = open {
        let (opener, closer) = if antisym { ("[", "]") } else { ("(", ")") };
        return Err(toks.error(format!(
            "grupo de simetrizacao aberto com `{opener}` nunca foi fechado com `{closer}`"
        )));
    }
    toks.expect_sym(']')?;
    Ok(ParsedFactor { head_name, indices, derivatives, sym_groups })
}

/// The factorial `k!`, the number of terms an order-`k` symmetrisation
/// expands to and hence its normalising denominator.
fn factorial(k: usize) -> i64 {
    (1..=k as i64).product::<i64>().max(1)
}

/// Every permutation of `0..k`, each paired with its sign.
fn permutations_with_sign(k: usize) -> Vec<(Vec<usize>, i64)> {
    let mut cur: Vec<usize> = (0..k).collect();
    let mut out = Vec::new();
    fn go(cur: &mut Vec<usize>, i: usize, out: &mut Vec<(Vec<usize>, i64)>) {
        if i == cur.len() {
            // Sign from the inversion count. `k` is a tensor rank, so
            // this quadratic count is over a handful of elements.
            let inv = (0..cur.len())
                .flat_map(|a| ((a + 1)..cur.len()).map(move |b| (a, b)))
                .filter(|(a, b)| cur[*a] > cur[*b])
                .count();
            out.push((cur.clone(), if inv % 2 == 0 { 1 } else { -1 }));
            return;
        }
        for j in i..cur.len() {
            cur.swap(i, j);
            go(cur, i + 1, out);
            cur.swap(i, j);
        }
    }
    go(&mut cur, 0, &mut out);
    out
}

/// Beyond this, a single symmetrisation would expand to more terms than
/// anyone can read (7! = 5040), so it is refused with its own message
/// rather than silently producing an unusable result.
const MAX_SYMMETRISATION_ORDER: usize = 6;

/// Expands every symmetrisation group in `parsed` into the sum it
/// abbreviates, returning one `(coefficient, per-factor index labels)`
/// pair per resulting monomial.
///
/// Groups multiply: each contributes its own permutations, and the
/// result is their Cartesian product across all factors of the monomial.
/// A factor with no groups contributes exactly one alternative, so the
/// no-symmetrisation case falls out as the singleton and costs nothing.
fn expand_symmetrisations(
    coeff: Scalar,
    parsed: &[ParsedFactor],
    toks: &TokStream,
) -> Result<Vec<(Scalar, Vec<Vec<String>>)>, CliError> {
    let mut out: Vec<(Scalar, Vec<Vec<String>>)> =
        vec![(coeff, parsed.iter().map(|f| f.indices.clone()).collect())];

    for (fi, factor) in parsed.iter().enumerate() {
        for group in &factor.sym_groups {
            if group.len > MAX_SYMMETRISATION_ORDER {
                return Err(toks.error(format!(
                    "simetrizacao sobre {} indices geraria {} termos; o maximo suportado e {MAX_SYMMETRISATION_ORDER} indices",
                    group.len,
                    factorial(group.len)
                )));
            }
            let perms = permutations_with_sign(group.len);
            let norm = Scalar::new(1, factorial(group.len));
            let mut next = Vec::with_capacity(out.len() * perms.len());
            for (c, labels) in &out {
                for (perm, sign) in &perms {
                    let mut labels = labels.clone();
                    let original: Vec<String> =
                        (0..group.len).map(|i| labels[fi][group.start + i].clone()).collect();
                    for i in 0..group.len {
                        labels[fi][group.start + i] = original[perm[i]].clone();
                    }
                    let signed = if group.antisym { Scalar::from_int(*sign) } else { Scalar::ONE };
                    next.push((*c * norm * signed, labels));
                }
            }
            out = next;
        }
    }
    Ok(out)
}

fn build_monomial(
    coeff: Scalar,
    parsed: Vec<ParsedFactor>,
    registry: &mut Registry,
) -> Result<Monomial, CliError> {
    let mut factors: SmallVec<[Factor; 4]> = SmallVec::new();
    let mut occurrences: HashMap<String, Vec<SlotId>> = HashMap::new();
    let mut head_ids: Vec<HeadId> = Vec::new();

    for (fi, pf) in parsed.iter().enumerate() {
        let base = registry.lookup_head(&pf.head_name)?;
        // `T[a,b;c]` is a factor of the *derivative* head, synthesized on
        // first use (see `Registry::derivative_head`) so that the same
        // derivative written in two places is the same head and the two
        // are comparable.
        let head = registry.derivative_head(base, pf.derivatives)?;
        let arity = registry.head(head).arity();
        if pf.indices.len() != arity {
            return Err(CliError::Parse {
                message: format!("`{}` has arity {arity}, but {} indices were given", pf.head_name, pf.indices.len()),
                position: None,
            });
        }
        head_ids.push(head);
        for (si, name) in pf.indices.iter().enumerate() {
            occurrences
                .entry(name.clone())
                .or_default()
                .push(SlotId { factor: fi as u16, slot: si as u8 });
        }
    }
    for head in head_ids {
        factors.push(Factor { head });
    }

    let mut free = Vec::new();
    let mut pairs = Vec::new();
    for (name, slots) in occurrences {
        match slots.as_slice() {
            [only] => free.push((*only, AbstractIndex::new(name))),
            [a, b] => pairs.push((*a, *b)),
            other => {
                return Err(CliError::Parse {
                    message: format!(
                        "index `{name}` appears {} times; it must appear once (free) or twice (contracted)",
                        other.len()
                    ),
                    position: None,
                })
            }
        }
    }

    let matching = Matching::try_new(pairs)?;
    Ok(Monomial::try_new(coeff, factors, matching, free, registry)?)
}

// ---------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------

/// Renders a monomial back to source-like text. Dummy edges have no name
/// in the representation (see `oderom-core`), so this invents fresh
/// letters for them on the way out -- a one-way, purely cosmetic
/// projection, not a "rename" of anything the data model stores.
/// Splits a sum of tensor monomials into its terms, each parsed by the
/// existing, already-tested [`parse_monomial`] -- deliberately *not* a
/// second expression grammar. The monomial grammar has no parentheses
/// and no infix operator other than juxtaposition (see
/// [`parse_monomial`]), so every `+`/`-` in the source is necessarily
/// at top level and a split on them is exact, not a heuristic that
/// could mis-handle nesting.
///
/// Each term keeps its own sign, since `parse_monomial` already accepts
/// a leading `+`/`-` and a rational coefficient (`-3/4 R[a,b]`). A `-`
/// can therefore never be confused with the `/` of a coefficient: `/`
/// only ever appears *between two integers* inside a coefficient, and
/// this split never looks inside one.
///
/// Empty input, or a trailing operator with nothing after it, is a
/// parse error rather than a silently-dropped term.
pub fn parse_polynomial(src: &str, registry: &mut Registry) -> Result<Vec<Monomial>, CliError> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for (i, ch) in src.char_indices() {
        // A sign starts a new term only if a *monomial* precedes it.
        // Nothing at all (start of input) or a bare sign already
        // collected (`... + -1 R[a,b]`, which the renderer itself emits
        // for a negative coefficient) both mean this sign belongs to the
        // term being built, which `parse_monomial` handles via its own
        // leading-sign and coefficient parsing.
        let pending = current.trim();
        if (ch == '+' || ch == '-') && !pending.is_empty() && pending != "+" && pending != "-" {
            terms.push(std::mem::take(&mut current));
            current.push(ch);
        } else {
            let _ = i;
            current.push(ch);
        }
    }
    if !current.trim().is_empty() {
        terms.push(current);
    }
    if terms.is_empty() {
        return Err(CliError::Parse { message: "expressao vazia: esperava ao menos um monomio tensorial".to_string(), position: None });
    }
    let mut parsed = Vec::with_capacity(terms.len());
    for term in &terms {
        // Fold any run of leading signs into one, so `+ -1 R[a,b]` --
        // which is exactly what this tool's own output looks like for a
        // negative coefficient, and therefore what a reader will paste
        // straight back in -- reaches `parse_monomial` as `- 1 R[a,b]`.
        // `parse_monomial` accepts a single leading sign, by design; the
        // doubling is an artifact of splitting a sum, so it is this
        // function's job to undo, not that grammar's to grow for.
        let mut negative = false;
        let mut rest = term.trim();
        loop {
            if let Some(r) = rest.strip_prefix('-') {
                negative = !negative;
                rest = r.trim_start();
            } else if let Some(r) = rest.strip_prefix('+') {
                rest = r.trim_start();
            } else {
                break;
            }
        }
        if rest.is_empty() {
            return Err(CliError::Parse { message: format!("operador `{}` sem monomio depois dele", term.trim()), position: None });
        }
        let normalized = if negative { format!("-{rest}") } else { rest.to_string() };
        // `extend`, not `push`: a term carrying a symmetrisation group
        // expands to several monomials, and the sum absorbs them all.
        parsed.extend(parse_monomial_expanded(&normalized, registry)?);
    }
    Ok(parsed)
}

pub fn format_monomial(m: &Monomial, registry: &Registry) -> String {
    let mut labels: HashMap<SlotId, String> = HashMap::new();
    for (slot, label) in m.free() {
        labels.insert(*slot, label.name().to_string());
    }

    let used: std::collections::HashSet<String> = labels.values().cloned().collect();
    // 'a'..'z', then 'a1', 'b1', .. once single letters run out -- no
    // fixed cap on how many dummy pairs a monomial may have.
    let mut candidates =
        (0u32..).flat_map(|round| ('a'..='z').map(move |c| if round == 0 { c.to_string() } else { format!("{c}{round}") }));
    let mut fresh = move || loop {
        let candidate = candidates.next().expect("infinite iterator");
        if !used.contains(&candidate) {
            return candidate;
        }
    };
    for &(a, b) in m.contractions().pairs() {
        let name = fresh();
        labels.insert(a, name.clone());
        labels.insert(b, name);
    }

    let mut out = String::new();
    if m.coeff() != Scalar::ONE || m.factors().is_empty() {
        out.push_str(&m.coeff().to_string());
        // A factorless monomial *is* its coefficient (`g[a,a]` with the
        // metric eliminated is the bare dimension), so no separator
        // follows -- otherwise the rendering carries a trailing space
        // and no longer round-trips through the parser.
        if !m.factors().is_empty() {
            out.push(' ');
        }
    }
    for (fi, factor) in m.factors().iter().enumerate() {
        let head = registry.head(factor.head);
        // A derivative head is stored under the synthesized name
        // `base;k`; it renders as the base name with its derivative
        // indices after a `;`, which is both the standard GR spelling
        // and exactly what `parse_factor` accepts -- so this output
        // parses straight back in.
        let derivatives = head.derivative_count();
        let base_arity = head.arity() - derivatives;
        out.push_str(head.name.split(';').next().unwrap_or(&head.name));
        out.push('[');
        for slot in 0..head.arity() {
            if slot == base_arity && derivatives > 0 {
                out.push(';');
            } else if slot > 0 {
                out.push(',');
            }
            let id = SlotId { factor: fi as u16, slot: slot as u8 };
            out.push_str(&labels[&id]);
        }
        out.push(']');
        if fi + 1 < m.factors().len() {
            out.push(' ');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_expr::Expr;

    /// `unwrap_err()` needs `T: Debug`, which `Model` deliberately does
    /// not derive (nothing else in this crate needs it) -- this is the
    /// same "expect an error, don't care what `Ok` would have looked
    /// like" check without adding that bound just for tests.
    fn expect_parse_error(src: &str) -> CliError {
        match parse_model(src) {
            Err(e) => e,
            Ok(_) => panic!("expected a parse error, got Ok"),
        }
    }

    const PRELUDE: &str = "
manifold M dim 4
bundle TM on M dim 4
head R : TM*, TM*, TM*, TM* symmetry (1 2)- (3 4)- (1 3)(2 4)+
head g : TM*, TM* symmetry (1 2)+
head eps : TM*, TM*, TM* symmetry antisymmetric
head W : TM*
";

    #[test]
    fn classify_block_recognizes_every_declaration_keyword() {
        for kw in ["manifold", "bundle", "head", "chart", "metric", "connection"] {
            assert_eq!(classify_block(&format!("{kw} whatever")).unwrap(), BlockKind::Declaration, "keyword: {kw}");
        }
    }

    #[test]
    fn classify_block_recognizes_every_query_keyword() {
        for kw in [
            "christoffel",
            "riemann",
            "ricci",
            "scalar",
            "kretschmann",
            "geodesic",
            "accel",
            "einstein",
            "riccisquare",
            "gaussbonnet",
            "weyl",
            "weylsquare",
        ] {
            assert_eq!(classify_block(kw).unwrap(), BlockKind::Query, "keyword: {kw}");
        }
    }

    #[test]
    fn classify_block_rejects_an_unrecognized_leading_keyword() {
        let err = classify_block("bogus stuff").unwrap_err();
        assert!(matches!(err, CliError::Parse { position: Some(_), .. }));
    }

    #[test]
    fn classify_block_rejects_an_empty_block() {
        assert!(classify_block("").is_err());
        assert!(classify_block("   \n  ").is_err());
    }

    #[test]
    fn classify_block_recognizes_an_alias_definition_as_a_declaration() {
        assert_eq!(classify_block("f := 1 - 2*M/r").unwrap(), BlockKind::Declaration);
    }

    #[test]
    fn classify_block_does_not_mistake_a_bare_colon_for_an_alias() {
        // `head NAME : SLOT` uses a bare `:` that is never immediately
        // followed by `=` -- must not be misread as an alias definition.
        assert_eq!(classify_block("head R : TM*, TM*").unwrap(), BlockKind::Declaration);
    }

    #[test]
    fn parse_model_expands_an_alias_inline_into_a_metric_component() {
        let src = format!(
            "{PRELUDE}
f := 1 - 2*M/r + Q^2/r^2
chart schw on M coords (t, r, theta, phi)
metric mg on schw bundle TM {{
  [t,t] = -f,
  [r,r] = 1/f,
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}}
"
        );
        let model = parse_model(&src).unwrap();
        assert_eq!(model.aliases.len(), 1);
        assert!(model.aliases.contains_key("f"));

        // Golden check: the SAME metric written out by hand, with `f`
        // substituted textually, must produce byte-identical stored
        // components -- proof the expansion is faithful, not just that
        // parsing succeeded.
        let src_no_alias = format!(
            "{PRELUDE}
chart schw on M coords (t, r, theta, phi)
metric mg on schw bundle TM {{
  [t,t] = -(1 - 2*M/r + Q^2/r^2),
  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}}
"
        );
        let model_no_alias = parse_model(&src_no_alias).unwrap();
        let (_, _, tensor_alias) = &model.metrics["mg"];
        let (_, _, tensor_direct) = &model_no_alias.metrics["mg"];
        assert_eq!(tensor_alias.canonical_hash(), tensor_direct.canonical_hash());
    }

    #[test]
    fn an_alias_can_reference_an_earlier_alias() {
        let src = format!(
            "{PRELUDE}
f := 1 - 2*M/r
g := f^2
chart schw on M coords (t, r, theta, phi)
metric g2 on schw bundle TM {{
  [t,t] = -g,
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
"
        );
        let model = parse_model(&src).unwrap();
        assert_eq!(model.aliases.len(), 2);
        // `g`'s own stored value must already be fully expanded (no
        // trace of `f` left inside it) -- confirmed by comparing against
        // the same expression built directly.
        let expected = oderom_expr::normalize(&(Expr::one() - Expr::int(2) * Expr::var("M") / Expr::var("r")).pow(2));
        assert_eq!(oderom_expr::normalize(&model.aliases["g"]), expected);
    }

    #[test]
    fn redeclaring_the_same_alias_name_is_a_clean_error() {
        let src = "f := 1\nf := 2\n";
        let err = expect_parse_error(src);
        let message = err.to_string();
        assert!(message.contains("alias `f` is already declared"), "{message}");
    }

    /// A metric's off-diagonal component may be declared through either
    /// index order -- `[t,phi]` and `[phi,t]` name the same component --
    /// and doing so with the *same* value on both sides is not the
    /// mistake `set_declared`'s conflict check exists to catch (see the
    /// next test for the case that is).
    #[test]
    fn declaring_an_off_diagonal_metric_component_both_ways_with_the_same_value_is_fine() {
        let src = format!(
            "{PRELUDE}
chart kerrlike on M coords (t, r, theta, phi)
metric km on kerrlike bundle TM {{
  [t,t] = -1,
  [t,phi] = M*r,
  [phi,t] = M*r,
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
"
        );
        parse_model(&src).expect("declaring the same off-diagonal value both ways must not error");
    }

    /// The bug this round's `ComponentTensor::set_declared` exists to
    /// prevent: `[t,phi]` and `[phi,t]` are the same symmetric
    /// component, so declaring them with two *different* values is
    /// contradictory, not "last write wins". Must be a clean parse
    /// error naming both components and both values, never a silently
    /// accepted metric nobody actually wrote.
    #[test]
    fn declaring_an_off_diagonal_metric_component_both_ways_with_conflicting_values_is_a_clean_error() {
        let src = format!(
            "{PRELUDE}
chart kerrlike on M coords (t, r, theta, phi)
metric km on kerrlike bundle TM {{
  [t,t] = -1,
  [t,phi] = M*r,
  [phi,t] = 2*M*r,
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
"
        );
        let err = expect_parse_error(&src);
        let message = err.to_string();
        assert!(message.contains("[0, 3]") && message.contains("[3, 0]"), "{message}");
        assert!(message.contains("M*r") && message.contains("2*M*r"), "{message}");
    }

    #[test]
    fn using_an_alias_before_its_declaration_is_a_clean_error_not_a_free_variable() {
        let src = format!(
            "{PRELUDE}
chart schw on M coords (t, r, theta, phi)
metric mg on schw bundle TM {{
  [t,t] = -f,
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
f := 1 - 2*M/r
"
        );
        let err = expect_parse_error(&src);
        let message = err.to_string();
        assert!(message.contains("used before its own `f := ...` declaration"), "{message}");
    }

    #[test]
    fn a_name_that_is_never_declared_as_an_alias_anywhere_stays_a_free_variable() {
        // Confirms end to end (through parse_model, not just
        // expr_parser's own unit test) that ordinary constants (M, Q,
        // ...) are completely unaffected by this feature.
        let src = format!(
            "{PRELUDE}
chart schw on M coords (t, r, theta, phi)
metric mg on schw bundle TM {{
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
"
        );
        assert!(parse_model(&src).is_ok());
    }

    #[test]
    fn parenthesized_use_of_an_alias_name_is_the_indeterminate_function_never_the_alias() {
        // Marco 6 step 4 (DESIGN-M6-PREP.md section 1) filled in the
        // parenthesized reservation this test used to check was still
        // empty: `f(r)` is no longer rejected -- it is the indeterminate
        // function `f` applied to `r`, completely independent of `f`'s
        // OWN alias declaration two lines above. End to end through
        // `parse_model` (not just `expr_parser`'s own unit test): the
        // metric's `[t,t]` component must contain `Func{name:"f",...}`,
        // never the alias's expanded `1 - 2*M/r`.
        let src = format!(
            "{PRELUDE}
f := 1 - 2*M/r
chart schw on M coords (t, r, theta, phi)
metric mg on schw bundle TM {{
  [t,t] = -f(r),
  [r,r] = 1,
  [theta,theta] = 1,
  [phi,phi] = 1
}}
"
        );
        let model = parse_model(&src).unwrap();
        let (_, _, tensor) = model.metrics.get("mg").expect("mg was declared");
        let tt = tensor.get(&model.registry, &[0, 0]).unwrap();
        assert!(matches!(tt, oderom_expr::Expr::Mul(_)), "expected -f(r) as a Mul, got {tt:?}");
        let contains_func = |e: &oderom_expr::Expr| -> bool {
            fn walk(e: &oderom_expr::Expr) -> bool {
                match e {
                    oderom_expr::Expr::Func { name, .. } => name == "f",
                    oderom_expr::Expr::Mul(fs) | oderom_expr::Expr::Add(fs) => fs.iter().any(walk),
                    oderom_expr::Expr::Pow(b, _) => walk(b),
                    _ => false,
                }
            }
            walk(e)
        };
        assert!(contains_func(&tt), "expected the metric component to contain the indeterminate function f, not the alias's expansion: {tt:?}");
    }

    #[test]
    fn a_name_colliding_with_a_reserved_keyword_never_silently_becomes_an_alias() {
        // `chart := 5` -- the leading-keyword arms in both `classify_block`
        // and `parse_model`'s own loop take priority over alias
        // recognition, so this is a real (if not maximally specific)
        // error from inside `parse_chart_decl`, never a silent alias.
        let err = expect_parse_error("chart := 5");
        assert!(matches!(err, CliError::Parse { .. }));
    }

    #[test]
    fn parse_query_accepts_a_bare_command() {
        let q = parse_query("ricci").unwrap();
        assert_eq!(
            q,
            Query::Command {
                name: Spanned::new(CommandName::Ricci, Span { start: Position { line: 1, column: 1 }, end: Position { line: 1, column: 6 } }),
                target: None,
                variance: None,
            }
        );
    }

    #[test]
    fn parse_query_accepts_a_command_with_a_target() {
        let q = parse_query("ricci g").unwrap();
        match q {
            Query::Command { name, target, variance } => {
                assert_eq!(name.value, CommandName::Ricci);
                assert_eq!(target.unwrap().value, "g");
                assert!(variance.is_none());
            }
            Query::Geodesic { .. } => panic!("expected Query::Command, got Query::Geodesic"),
            Query::Accel { .. } => panic!("expected Query::Command, got Query::Accel"),
            Query::Export { .. } => panic!("expected Query::Command, got Query::Export"),
        }
    }

    #[test]
    fn parse_query_rejects_an_unknown_command() {
        let err = parse_query("bogus").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("unknown command"));
    }

    /// `geodesic PARAM` -- parameter only, target resolved the same
    /// convention every other command's optional target already uses
    /// (the file's sole metric/connection).
    #[test]
    fn parse_query_accepts_geodesic_with_only_a_parameter() {
        let q = parse_query("geodesic tau").unwrap();
        match q {
            Query::Geodesic { target, param } => {
                assert!(target.is_none());
                assert_eq!(param.value, "tau");
            }
            Query::Command { .. } => panic!("expected Query::Geodesic, got Query::Command"),
            Query::Accel { .. } => panic!("expected Query::Geodesic, got Query::Accel"),
            Query::Export { .. } => panic!("expected Query::Geodesic, got Query::Export"),
        }
    }

    /// `geodesic TARGET PARAM` -- target then parameter, in that order
    /// ("g's geodesic equation, parametrized by tau").
    #[test]
    fn parse_query_accepts_geodesic_with_a_target_and_a_parameter() {
        let q = parse_query("geodesic g tau").unwrap();
        match q {
            Query::Geodesic { target, param } => {
                assert_eq!(target.unwrap().value, "g");
                assert_eq!(param.value, "tau");
            }
            Query::Command { .. } => panic!("expected Query::Geodesic, got Query::Command"),
            Query::Accel { .. } => panic!("expected Query::Geodesic, got Query::Accel"),
            Query::Export { .. } => panic!("expected Query::Geodesic, got Query::Export"),
        }
    }

    /// The parameter is mandatory -- unlike the other five commands'
    /// optional target, `geodesic` alone (no parameter at all) is a
    /// clear parse error, never a query with an implicit/guessed name.
    #[test]
    fn parse_query_rejects_geodesic_with_no_parameter() {
        let err = parse_query("geodesic").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("affine parameter"));
    }

    #[test]
    fn parse_query_rejects_geodesic_with_trailing_input() {
        let err = parse_query("geodesic g tau extra").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("extra input"));
    }

    /// `accel` shares its grammar with `geodesic` (same tail-parsing
    /// routine, `parse_geodesic_like_query_tail`) -- these mirror
    /// `geodesic`'s own four grammar tests above, proving the shared
    /// routine really does build the right variant for each keyword,
    /// not just whichever one happened to be tested first.
    #[test]
    fn parse_query_accepts_accel_with_only_a_parameter() {
        let q = parse_query("accel tau").unwrap();
        match q {
            Query::Accel { target, param } => {
                assert!(target.is_none());
                assert_eq!(param.value, "tau");
            }
            Query::Command { .. } => panic!("expected Query::Accel, got Query::Command"),
            Query::Geodesic { .. } => panic!("expected Query::Accel, got Query::Geodesic"),
            Query::Export { .. } => panic!("expected Query::Accel, got Query::Export"),
        }
    }

    #[test]
    fn parse_query_accepts_accel_with_a_target_and_a_parameter() {
        let q = parse_query("accel g tau").unwrap();
        match q {
            Query::Accel { target, param } => {
                assert_eq!(target.unwrap().value, "g");
                assert_eq!(param.value, "tau");
            }
            Query::Command { .. } => panic!("expected Query::Accel, got Query::Command"),
            Query::Geodesic { .. } => panic!("expected Query::Accel, got Query::Geodesic"),
            Query::Export { .. } => panic!("expected Query::Accel, got Query::Export"),
        }
    }

    #[test]
    fn parse_query_rejects_accel_with_no_parameter() {
        let err = parse_query("accel").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("affine parameter"));
    }

    #[test]
    fn parse_query_rejects_accel_with_trailing_input() {
        let err = parse_query("accel g tau extra").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("extra input"));
    }

    #[test]
    fn parse_query_rejects_trailing_input() {
        let err = parse_query("ricci g extra").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("extra input"));
    }

    // -------------------------------------------------------------
    // Index-variance marker grammar (Rodada Variancia): `[up,down,...]`.
    // Whether a given command *accepts* a marker at all (Christoffel,
    // the five scalars) and arity-against-rank are validated downstream
    // (`oderom-session::run::run_command_query`), not by this grammar --
    // these tests only check parsing shape.
    // -------------------------------------------------------------

    #[test]
    fn parse_query_accepts_a_bare_command_with_a_variance_marker_and_no_target() {
        let q = parse_query("riemann [up,down,down,down]").unwrap();
        match q {
            Query::Command { name, target, variance } => {
                assert_eq!(name.value, CommandName::Riemann);
                assert!(target.is_none());
                assert_eq!(variance.unwrap().value, vec![Variance::Contra, Variance::Co, Variance::Co, Variance::Co]);
            }
            other => panic!("expected Query::Command, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_accepts_a_target_then_a_variance_marker_in_that_order() {
        let q = parse_query("ricci g [up,up]").unwrap();
        match q {
            Query::Command { name, target, variance } => {
                assert_eq!(name.value, CommandName::Ricci);
                assert_eq!(target.unwrap().value, "g");
                assert_eq!(variance.unwrap().value, vec![Variance::Contra, Variance::Contra]);
            }
            other => panic!("expected Query::Command, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_with_no_bracket_leaves_variance_none() {
        let q = parse_query("riemann g").unwrap();
        match q {
            Query::Command { variance, .. } => assert!(variance.is_none(), "no bracket in the input must mean no variance marker, not an empty one"),
            other => panic!("expected Query::Command, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_rejects_a_word_other_than_up_or_down_inside_the_bracket() {
        let err = parse_query("riemann [up,sideways,down,down]").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("up") && format!("{err}").contains("down"), "{err}");
    }

    #[test]
    fn parse_query_rejects_an_unclosed_variance_bracket() {
        let err = parse_query("riemann [up,down").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
    }

    #[test]
    fn parse_query_rejects_trailing_input_after_a_variance_marker() {
        let err = parse_query("riemann [up,down,down,down] extra").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("extra input"));
    }

    // -------------------------------------------------------------
    // `export` (Rodada Exportação)
    // -------------------------------------------------------------

    #[test]
    fn parse_query_accepts_export_sympy_of_a_bare_command() {
        let q = parse_query("export sympy kretschmann").unwrap();
        match q {
            Query::Export { format, inner } => {
                assert_eq!(format.value, Target::Sympy);
                assert!(matches!(*inner, Query::Command { name, .. } if name.value == CommandName::Kretschmann));
            }
            other => panic!("expected Query::Export, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_accepts_export_mathematica_of_a_command_with_a_target() {
        let q = parse_query("export mathematica riemann g").unwrap();
        match q {
            Query::Export { format, inner } => {
                assert_eq!(format.value, Target::Mathematica);
                match *inner {
                    Query::Command { name, target, .. } => {
                        assert_eq!(name.value, CommandName::Riemann);
                        assert_eq!(target.unwrap().value, "g");
                    }
                    other => panic!("expected Query::Command, got {other:?}"),
                }
            }
            other => panic!("expected Query::Export, got {other:?}"),
        }
    }

    /// `export` composes with every other query shape, not just a bare
    /// command -- geodesic here, since it has its own distinct grammar
    /// tail (`parse_geodesic_like_query_tail`), the one most likely to
    /// interact badly with a wrapping production if the recursion were
    /// done wrong.
    #[test]
    fn parse_query_accepts_export_of_a_geodesic_query() {
        let q = parse_query("export sympy geodesic g tau").unwrap();
        match q {
            Query::Export { format, inner } => {
                assert_eq!(format.value, Target::Sympy);
                match *inner {
                    Query::Geodesic { target, param } => {
                        assert_eq!(target.unwrap().value, "g");
                        assert_eq!(param.value, "tau");
                    }
                    other => panic!("expected Query::Geodesic, got {other:?}"),
                }
            }
            other => panic!("expected Query::Export, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_accepts_export_of_a_variance_marked_query() {
        let q = parse_query("export mathematica riemann [up,down,down,down]").unwrap();
        match q {
            Query::Export { inner, .. } => {
                assert!(matches!(*inner, Query::Command { variance: Some(_), .. }));
            }
            other => panic!("expected Query::Export, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_rejects_an_unknown_export_target() {
        let err = parse_query("export octave kretschmann").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        let message = format!("{err}");
        assert!(message.contains("unknown export target"));
        assert!(message.contains("mathematica"));
        assert!(message.contains("sympy"));
    }

    #[test]
    fn parse_query_rejects_export_with_no_target_and_no_query() {
        let err = parse_query("export").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
    }

    #[test]
    fn parse_query_rejects_export_with_a_target_but_no_query() {
        let err = parse_query("export sympy").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("needs a query to export"));
    }

    #[test]
    fn parse_query_rejects_nested_export() {
        let err = parse_query("export sympy export mathematica kretschmann").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("cannot be nested"));
    }

    /// `geodesic`, not a bare command, is the one production whose own
    /// grammar has no room for an extra trailing identifier to be
    /// mistaken for anything legitimate (a bare command like
    /// `kretschmann` would instead read a trailing word as its own
    /// optional target -- not a useful test of trailing-input
    /// rejection).
    #[test]
    fn parse_query_rejects_trailing_input_after_export() {
        let err = parse_query("export sympy geodesic g tau extra").unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
        assert!(format!("{err}").contains("extra input"));
    }

    #[test]
    fn classify_block_recognizes_export_as_a_query() {
        assert_eq!(classify_block("export sympy kretschmann").unwrap(), BlockKind::Query);
    }

    /// The whole reason `TokStream` tracks positions: a real, multi-line
    /// `.od` source with an error on a specific line reports THAT line,
    /// not line 1 or some other approximation.
    #[test]
    fn parse_errors_report_the_correct_line_and_column() {
        let src = "manifold M dim 4\nbundle TM on M dim 4\nbogus keyword here\n";
        let err = match parse_model(src) {
            Err(e) => e,
            Ok(_) => panic!("expected a parse error"),
        };
        let CliError::Parse { position, .. } = err else { panic!("expected CliError::Parse, got {err:?}") };
        assert_eq!(position, Some(Position { line: 3, column: 1 }));
    }

    #[test]
    fn parses_prelude_declarations() {
        let reg = parse_model(PRELUDE).unwrap().registry;
        assert_eq!(reg.manifold(reg.lookup_manifold("M").unwrap()).dim, 4);
        let r = reg.lookup_head("R").unwrap();
        assert_eq!(reg.head(r).arity(), 4);
        assert_eq!(reg.head(r).symmetry.order(), 8);
        let eps = reg.lookup_head("eps").unwrap();
        assert_eq!(reg.head(eps).symmetry.order(), 6);
        let w = reg.lookup_head("W").unwrap();
        assert_eq!(reg.head(w).arity(), 1);
        assert_eq!(reg.head(w).symmetry.order(), 1);
    }

    #[test]
    fn parses_and_resolves_a_monomial() {
        let mut reg = parse_model(PRELUDE).unwrap().registry;
        let m = parse_monomial("R[a,b,c,d] R[c,d,a,b]", &mut reg).unwrap();
        assert_eq!(m.factors().len(), 2);
        assert!(m.free().is_empty());
        assert_eq!(m.contractions().len(), 4);
    }

    #[test]
    fn parses_rational_coefficient_with_sign() {
        let mut reg = parse_model(PRELUDE).unwrap().registry;
        let m = parse_monomial("-3/4 g[a,b]", &mut reg).unwrap();
        assert_eq!(m.coeff(), Scalar::new(-3, 4));
    }

    #[test]
    fn index_appearing_three_times_is_a_parse_error() {
        let mut reg = parse_model(PRELUDE).unwrap().registry;
        let err = parse_monomial("R[a,a,a,b]", &mut reg).unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
    }

    #[test]
    fn wrong_arity_is_a_parse_error() {
        let mut reg = parse_model(PRELUDE).unwrap().registry;
        let err = parse_monomial("g[a,b,c]", &mut reg).unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));
    }

    #[test]
    fn format_round_trips_free_indices() {
        let mut reg = parse_model(PRELUDE).unwrap().registry;
        let m = parse_monomial("R[a,b,c,d]", &mut reg).unwrap();
        assert_eq!(format_monomial(&m, &reg), "R[a,b,c,d]");
    }
}
