//! Resolving a `metric`/`connection` component line's index list against
//! a [`Chart`]'s concrete coordinate positions (DESIGN-UI.md 6.3/6.3b).
//! Two spellings, both landing on the same `Vec<u8>` of 0-based
//! positions:
//!
//! - ASCII, always comma-separated, never ambiguous: `[t,r]`.
//! - LaTeX, `NAME_{...}` where `NAME` echoes the declaration's own name
//!   (`g_{tt}`): comma-separated inside the braces works exactly like
//!   the bracket form, but a *glued* run (`tt`, `rhor`) is decomposed by
//!   backtracking search over the chart's declared coordinate names --
//!   never maximal munch, so a chart with both `r` and `rho` still finds
//!   `rhor` -> `[rho, r]` even though greedily matching `r` first would
//!   dead-end one character short. Exactly one full decomposition of the
//!   expected length is required: zero is a parse error naming the
//!   coordinates that were available, two or more is a parse error
//!   listing every reading and pointing at the comma form, which always
//!   works, in any chart, as the unambiguous escape hatch.
//!
//! This module only ever resolves *concrete* coordinate indices. Marco
//! 1's abstract tensor-index syntax (`R[a,b,c,d]`, parsed by
//! `parser::parse_monomial`) is a separate grammar that never consults a
//! `Chart` at all -- see DESIGN-UI.md 6.3b for why the two can never be
//! confused with each other, by construction, not by looking at names.

use crate::error::CliError;
use crate::expr_parser::parse_scalar_expr;
use crate::parser::{Tok, TokStream};
use oderom_components::Chart;
use oderom_expr::Expr;

/// One line of a component block: `[i1,i2,...] = EXPR` or
/// `NAME_{...} = EXPR`, where `NAME` must equal `decl_name`.
pub(crate) fn parse_component_line(
    toks: &mut TokStream,
    chart: &Chart,
    arity: usize,
    decl_name: &str,
) -> Result<(Vec<u8>, Expr), CliError> {
    let indices = match toks.peek().clone() {
        Tok::Sym('[') => parse_bracket_indices(toks, chart, arity)?,
        Tok::Ident(name) if name == decl_name => {
            toks.advance();
            parse_subscript_indices(toks, chart, arity)?
        }
        other => {
            return Err(CliError::Parse(format!(
                "expected `[...]` or `{decl_name}_{{...}}`, found {other:?}"
            )))
        }
    };
    toks.expect_sym('=')?;
    let expr = parse_scalar_expr(toks)?;
    Ok((indices, expr))
}

fn parse_bracket_indices(toks: &mut TokStream, chart: &Chart, arity: usize) -> Result<Vec<u8>, CliError> {
    toks.expect_sym('[')?;
    let mut out = Vec::new();
    loop {
        out.push(resolve_one_index_token(toks, chart)?);
        if *toks.peek() == Tok::Sym(',') {
            toks.advance();
        } else {
            break;
        }
    }
    toks.expect_sym(']')?;
    check_arity(&out, arity)?;
    Ok(out)
}

fn resolve_one_index_token(toks: &mut TokStream, chart: &Chart) -> Result<u8, CliError> {
    match toks.advance() {
        Tok::Int(n) => Ok(n as u8),
        Tok::Ident(name) => coord_position(chart, &name)
            .ok_or_else(|| CliError::Parse(format!("`{name}` is not a coordinate of this chart ({})", coord_list(chart)))),
        Tok::Command(name) => {
            let lower = name.to_ascii_lowercase();
            coord_position(chart, &lower)
                .ok_or_else(|| CliError::Parse(format!("`\\{name}` is not a coordinate of this chart ({})", coord_list(chart))))
        }
        other => Err(CliError::Parse(format!("expected a coordinate name or integer index, found {other:?}"))),
    }
}

fn parse_subscript_indices(toks: &mut TokStream, chart: &Chart, arity: usize) -> Result<Vec<u8>, CliError> {
    toks.expect_sym('_')?;
    toks.expect_sym('{')?;
    let mut pieces: Vec<Tok> = Vec::new();
    loop {
        match toks.peek().clone() {
            Tok::Sym('}') => {
                toks.advance();
                break;
            }
            Tok::Eof => return Err(CliError::Parse("unterminated `_{...}`".to_string())),
            t => {
                pieces.push(t);
                toks.advance();
            }
        }
    }

    if pieces.contains(&Tok::Sym(',')) {
        let mut out = Vec::new();
        for group in pieces.split(|t| *t == Tok::Sym(',')) {
            let position = match group {
                [Tok::Int(n)] => *n as u8,
                [Tok::Ident(name)] => coord_position(chart, name)
                    .ok_or_else(|| CliError::Parse(format!("`{name}` is not a coordinate of this chart ({})", coord_list(chart))))?,
                [Tok::Command(name)] => {
                    let lower = name.to_ascii_lowercase();
                    coord_position(chart, &lower)
                        .ok_or_else(|| CliError::Parse(format!("`\\{name}` is not a coordinate of this chart ({})", coord_list(chart))))?
                }
                _ => return Err(CliError::Parse("expected one coordinate name or integer between commas".to_string())),
            };
            out.push(position);
        }
        check_arity(&out, arity)?;
        Ok(out)
    } else {
        decompose_glued(&pieces, chart, arity)
    }
}

/// The core of 6.3: every full decomposition of the glued `pieces` into
/// the chart's declared coordinate names, backtracking (not maximal
/// munch), required to be exactly one reading of length `arity`.
fn decompose_glued(pieces: &[Tok], chart: &Chart, arity: usize) -> Result<Vec<u8>, CliError> {
    let mut per_piece: Vec<Vec<Vec<String>>> = Vec::with_capacity(pieces.len());
    for p in pieces {
        match p {
            Tok::Command(name) => {
                let lower = name.to_ascii_lowercase();
                if chart.coords.contains(&lower) {
                    per_piece.push(vec![vec![lower]]);
                } else {
                    return Err(CliError::Parse(format!(
                        "`\\{name}` is not a coordinate of this chart ({})",
                        coord_list(chart)
                    )));
                }
            }
            Tok::Ident(run) => per_piece.push(decompose_run(run, &chart.coords)),
            other => return Err(CliError::Parse(format!("expected a coordinate name, found {other:?}"))),
        }
    }

    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for candidates in &per_piece {
        let mut next = Vec::new();
        for existing in &combos {
            for candidate in candidates {
                if existing.len() + candidate.len() > arity {
                    continue; // prune: can never reach exactly `arity`
                }
                let mut merged = existing.clone();
                merged.extend(candidate.iter().cloned());
                next.push(merged);
            }
        }
        combos = next;
    }

    let mut readings: Vec<Vec<String>> = combos.into_iter().filter(|c| c.len() == arity).collect();
    readings.sort();
    readings.dedup();

    let glued_text: String = pieces.iter().map(token_text).collect();
    match readings.as_slice() {
        [] => Err(CliError::Parse(format!(
            "`{glued_text}` is not decomposable into the coordinates of this chart ({}); use the comma form to be explicit",
            coord_list(chart)
        ))),
        [reading] => {
            Ok(reading.iter().map(|name| coord_position(chart, name).expect("name came from chart.coords")).collect())
        }
        many => {
            let listed = many.iter().map(|r| format!("[{}]", r.join(","))).collect::<Vec<_>>().join(" or ");
            Err(CliError::Parse(format!(
                "`{glued_text}` is ambiguous: could be {listed} -- use the comma form to disambiguate"
            )))
        }
    }
}

/// All full decompositions of `s` into a sequence of `coords` entries.
/// Backtracking, not maximal munch: with `coords = ["r", "rho"]`, `"rhor"`
/// must still find `["rho", "r"]` after trying `"r"` first and dead-ending.
fn decompose_run(s: &str, coords: &[String]) -> Vec<Vec<String>> {
    fn rec(s: &str, coords: &[String], acc: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        if s.is_empty() {
            out.push(acc.clone());
            return;
        }
        for c in coords {
            if let Some(rest) = s.strip_prefix(c.as_str()) {
                acc.push(c.clone());
                rec(rest, coords, acc, out);
                acc.pop();
            }
        }
    }
    let mut acc = Vec::new();
    let mut out = Vec::new();
    rec(s, coords, &mut acc, &mut out);
    out
}

fn coord_position(chart: &Chart, name: &str) -> Option<u8> {
    chart.coords.iter().position(|c| c == name).map(|i| i as u8)
}

fn coord_list(chart: &Chart) -> String {
    chart.coords.join(", ")
}

fn check_arity(indices: &[u8], expected: usize) -> Result<(), CliError> {
    if indices.len() == expected {
        Ok(())
    } else {
        Err(CliError::Parse(format!("expected {expected} indices, found {}", indices.len())))
    }
}

fn token_text(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Command(s) => format!("\\{s}"),
        Tok::Int(n) => n.to_string(),
        Tok::Sym(c) => c.to_string(),
        Tok::Eof => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(coords: &[&str]) -> Chart {
        Chart::new(coords.iter().map(|s| s.to_string()))
    }

    #[test]
    fn glued_unique_reading_is_accepted() {
        let c = chart(&["t", "r", "theta", "phi"]);
        let mut toks = TokStream::new("_{tt}").unwrap();
        assert_eq!(parse_subscript_indices(&mut toks, &c, 2).unwrap(), vec![0, 0]);
    }

    #[test]
    fn backtracking_finds_the_longer_coordinate_after_the_short_one_fails() {
        // "r" alone would greedily match first and leave "hor" stranded;
        // backtracking must still find "rho" + "r".
        let c = chart(&["r", "rho"]);
        let mut toks = TokStream::new("_{rhor}").unwrap();
        assert_eq!(parse_subscript_indices(&mut toks, &c, 2).unwrap(), vec![1, 0]);
    }

    #[test]
    fn zero_decompositions_is_a_clear_error() {
        let c = chart(&["t", "r"]);
        let mut toks = TokStream::new("_{xyz}").unwrap();
        let err = parse_subscript_indices(&mut toks, &c, 2).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not decomposable"), "{msg}");
    }

    #[test]
    fn ambiguous_decomposition_lists_every_reading() {
        // "abcd" splits two different ways into exactly 2 coordinate
        // names: ["ab","cd"] and ["abc","d"] -- a real tie, not just a
        // wrong-length decomposition getting filtered out.
        let c = chart(&["ab", "cd", "abc", "d"]);
        let mut toks = TokStream::new("_{abcd}").unwrap();
        let err = parse_subscript_indices(&mut toks, &c, 2).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ambiguous"), "{msg}");
        assert!(msg.contains("[ab,cd]"), "{msg}");
        assert!(msg.contains("[abc,d]"), "{msg}");
    }

    #[test]
    fn comma_form_always_works_even_with_multi_character_coordinates() {
        let c = chart(&["u1", "v1"]);
        let mut toks = TokStream::new("_{u1,v1}").unwrap();
        assert_eq!(parse_subscript_indices(&mut toks, &c, 2).unwrap(), vec![0, 1]);
    }

    #[test]
    fn bracket_form_is_always_comma_separated() {
        let c = chart(&["t", "r"]);
        let mut toks = TokStream::new("[t,r]").unwrap();
        assert_eq!(parse_bracket_indices(&mut toks, &c, 2).unwrap(), vec![0, 1]);
    }

    #[test]
    fn greek_macro_in_glued_subscript_is_never_ambiguous() {
        let c = chart(&["t", "theta", "phi"]);
        let mut toks = TokStream::new(r"_{t\theta}").unwrap();
        assert_eq!(parse_subscript_indices(&mut toks, &c, 2).unwrap(), vec![0, 1]);
    }
}
