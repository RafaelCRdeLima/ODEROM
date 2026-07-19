//! Minimal hand-written recursive-descent parsing for `prelude.od` and for
//! the tensor-expression argument to `oderom canon`. No parser-generator
//! or new dependency: the grammar is small enough that a lexer plus a
//! handful of `parse_*` functions is clearer than pulling in a library.

use crate::error::CliError;
use oderom_core::{
    AbstractIndex, Factor, HeadId, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId,
    SlotSig, Variance,
};
use smallvec::SmallVec;
use std::collections::HashMap;

// ---------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Int(u64),
    Sym(char),
    Eof,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer { chars: src.chars().peekable() }
    }

    fn next_tok(&mut self) -> Result<Tok, CliError> {
        loop {
            match self.chars.peek() {
                None => return Ok(Tok::Eof),
                Some(c) if c.is_whitespace() => {
                    self.chars.next();
                }
                Some('#') => {
                    while let Some(&c) = self.chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.chars.next();
                    }
                }
                _ => break,
            }
        }
        match self.chars.peek().copied() {
            None => Ok(Tok::Eof),
            Some(c) if c.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(&d) = self.chars.peek() {
                    if d.is_ascii_digit() {
                        n.push(d);
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                n.parse().map(Tok::Int).map_err(|_| CliError::Parse(format!("bad integer `{n}`")))
            }
            Some(c) if c.is_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(&d) = self.chars.peek() {
                    if d.is_alphanumeric() || d == '_' {
                        s.push(d);
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                Ok(Tok::Ident(s))
            }
            Some(c) => {
                self.chars.next();
                Ok(Tok::Sym(c))
            }
        }
    }
}

/// Turns a source string into a token stream with one token of lookahead.
struct TokStream {
    toks: Vec<Tok>,
    pos: usize,
}

impl TokStream {
    fn new(src: &str) -> Result<Self, CliError> {
        let mut lexer = Lexer::new(src);
        let mut toks = Vec::new();
        loop {
            let t = lexer.next_tok()?;
            let done = t == Tok::Eof;
            toks.push(t);
            if done {
                break;
            }
        }
        Ok(TokStream { toks, pos: 0 })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn expect_sym(&mut self, c: char) -> Result<(), CliError> {
        match self.advance() {
            Tok::Sym(s) if s == c => Ok(()),
            other => Err(CliError::Parse(format!("expected `{c}`, found {other:?}"))),
        }
    }

    fn expect_ident(&mut self, kw: &str) -> Result<(), CliError> {
        match self.advance() {
            Tok::Ident(s) if s == kw => Ok(()),
            other => Err(CliError::Parse(format!("expected `{kw}`, found {other:?}"))),
        }
    }

    fn ident(&mut self) -> Result<String, CliError> {
        match self.advance() {
            Tok::Ident(s) => Ok(s),
            other => Err(CliError::Parse(format!("expected an identifier, found {other:?}"))),
        }
    }

    fn int(&mut self) -> Result<u64, CliError> {
        match self.advance() {
            Tok::Int(n) => Ok(n),
            other => Err(CliError::Parse(format!("expected a number, found {other:?}"))),
        }
    }
}

// ---------------------------------------------------------------------
// prelude.od: `manifold`, `bundle`, `head` declarations
// ---------------------------------------------------------------------

/// Parses a `prelude.od` source into a populated [`Registry`].
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
/// ```
pub fn parse_prelude(src: &str) -> Result<Registry, CliError> {
    let mut toks = TokStream::new(src)?;
    let mut reg = Registry::new();

    loop {
        match toks.peek().clone() {
            Tok::Eof => break,
            Tok::Ident(kw) if kw == "manifold" => {
                toks.advance();
                let name = toks.ident()?;
                toks.expect_ident("dim")?;
                let dim = toks.int()? as u32;
                reg.declare_manifold(&name, dim)?;
            }
            Tok::Ident(kw) if kw == "bundle" => {
                toks.advance();
                let name = toks.ident()?;
                toks.expect_ident("on")?;
                let manifold_name = toks.ident()?;
                let manifold = reg.lookup_manifold(&manifold_name)?;
                toks.expect_ident("dim")?;
                let dim = toks.int()? as u32;
                reg.declare_bundle(&name, manifold, dim)?;
            }
            Tok::Ident(kw) if kw == "head" => {
                toks.advance();
                parse_head_decl(&mut toks, &mut reg)?;
            }
            other => return Err(CliError::Parse(format!("expected a declaration, found {other:?}"))),
        }
    }

    Ok(reg)
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
                    apply_cycle(&mut images, &cycle, arity)?;
                    if *toks.peek() != Tok::Sym('(') {
                        break;
                    }
                }
                let sign = match toks.advance() {
                    Tok::Sym('+') => 1,
                    Tok::Sym('-') => -1,
                    other => {
                        return Err(CliError::Parse(format!(
                            "expected `+` or `-` after a symmetry generator's cycles, found {other:?}"
                        )))
                    }
                };
                gens.push(SignedPerm::new(Perm::from_images(images), sign));
            }
            _ => break,
        }
    }
    Ok(gens)
}

fn apply_cycle(images: &mut [u16], cycle: &[u16], arity: usize) -> Result<(), CliError> {
    for &c in cycle {
        if c == 0 || c as usize > arity {
            return Err(CliError::Parse(format!(
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
}

/// Parses and resolves a single tensor monomial against `registry`:
/// coefficient, factors by juxtaposition, and index names in brackets.
/// An index name appearing exactly twice across the whole monomial
/// becomes a contraction; exactly once, a free index; any other count is
/// a parse error.
pub fn parse_monomial(src: &str, registry: &Registry) -> Result<Monomial, CliError> {
    let mut toks = TokStream::new(src)?;

    let mut coeff_sign = 1i64;
    if *toks.peek() == Tok::Sym('-') {
        toks.advance();
        coeff_sign = -1;
    } else if *toks.peek() == Tok::Sym('+') {
        toks.advance();
    }

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
            other => return Err(CliError::Parse(format!("expected a tensor factor, found {other:?}"))),
        }
    }
    if factors.is_empty() {
        return Err(CliError::Parse("expected at least one tensor factor".to_string()));
    }

    build_monomial(coeff, factors, registry)
}

fn parse_factor(toks: &mut TokStream) -> Result<ParsedFactor, CliError> {
    let head_name = toks.ident()?;
    toks.expect_sym('[')?;
    let mut indices = Vec::new();
    loop {
        indices.push(toks.ident()?);
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    toks.expect_sym(']')?;
    Ok(ParsedFactor { head_name, indices })
}

fn build_monomial(
    coeff: Scalar,
    parsed: Vec<ParsedFactor>,
    registry: &Registry,
) -> Result<Monomial, CliError> {
    let mut factors: SmallVec<[Factor; 4]> = SmallVec::new();
    let mut occurrences: HashMap<String, Vec<SlotId>> = HashMap::new();
    let mut head_ids: Vec<HeadId> = Vec::new();

    for (fi, pf) in parsed.iter().enumerate() {
        let head = registry.lookup_head(&pf.head_name)?;
        let arity = registry.head(head).arity();
        if pf.indices.len() != arity {
            return Err(CliError::Parse(format!(
                "`{}` has arity {arity}, but {} indices were given",
                pf.head_name,
                pf.indices.len()
            )));
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
                return Err(CliError::Parse(format!(
                    "index `{name}` appears {} times; it must appear once (free) or twice (contracted)",
                    other.len()
                )))
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
    if m.coeff() != Scalar::ONE {
        out.push_str(&m.coeff().to_string());
        out.push(' ');
    }
    for (fi, factor) in m.factors().iter().enumerate() {
        let head = registry.head(factor.head);
        out.push_str(&head.name);
        out.push('[');
        for slot in 0..head.arity() {
            if slot > 0 {
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

    const PRELUDE: &str = "
manifold M dim 4
bundle TM on M dim 4
head R : TM*, TM*, TM*, TM* symmetry (1 2)- (3 4)- (1 3)(2 4)+
head g : TM*, TM* symmetry (1 2)+
head eps : TM*, TM*, TM* symmetry antisymmetric
head W : TM*
";

    #[test]
    fn parses_prelude_declarations() {
        let reg = parse_prelude(PRELUDE).unwrap();
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
        let reg = parse_prelude(PRELUDE).unwrap();
        let m = parse_monomial("R[a,b,c,d] R[c,d,a,b]", &reg).unwrap();
        assert_eq!(m.factors().len(), 2);
        assert!(m.free().is_empty());
        assert_eq!(m.contractions().len(), 4);
    }

    #[test]
    fn parses_rational_coefficient_with_sign() {
        let reg = parse_prelude(PRELUDE).unwrap();
        let m = parse_monomial("-3/4 g[a,b]", &reg).unwrap();
        assert_eq!(m.coeff(), Scalar::new(-3, 4));
    }

    #[test]
    fn index_appearing_three_times_is_a_parse_error() {
        let reg = parse_prelude(PRELUDE).unwrap();
        let err = parse_monomial("R[a,a,a,b]", &reg).unwrap_err();
        assert!(matches!(err, CliError::Parse(_)));
    }

    #[test]
    fn wrong_arity_is_a_parse_error() {
        let reg = parse_prelude(PRELUDE).unwrap();
        let err = parse_monomial("g[a,b,c]", &reg).unwrap_err();
        assert!(matches!(err, CliError::Parse(_)));
    }

    #[test]
    fn format_round_trips_free_indices() {
        let reg = parse_prelude(PRELUDE).unwrap();
        let m = parse_monomial("R[a,b,c,d]", &reg).unwrap();
        assert_eq!(format_monomial(&m, &reg), "R[a,b,c,d]");
    }
}
