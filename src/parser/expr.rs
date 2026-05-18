//! Parser for the RPM `%if` / `%elif` expression grammar.
//!
//! See [`crate::ast::expr`] for the grammar productions. This module
//! implements them as straightforward nom recursive-descent
//! combinators. The parser is *opportunistic*: when it fails on a
//! fragment of the modelled grammar (arithmetic, unmodelled
//! operators, …), the caller falls back to storing the line as
//! [`crate::ast::CondExpr::Raw`].
//!
//! Spans on every node are produced via [`span_between`] from the
//! cursor positions before and after each combinator.
//!
//! Whitespace handling: every combinator consumes leading whitespace
//! via [`space0`] but never trailing — the outer combinator is
//! responsible for handling the inter-operator gap.
//!
//! ## Diagnostics
//!
//! This layer emits no [`crate::parse_result::Diagnostic`]s. Parse failures bubble
//! up as `nom` errors and the caller in [`super::cond`] records the
//! line as [`crate::ast::CondExpr::Raw`]. A future
//! `W_IF_EXPR_FALLBACK` warning would require threading
//! [`super::state::ParserState`] through the sub-grammar; tracked as
//! a follow-up.

use nom::{
    IResult, Parser,
    bytes::complete::take_while1,
    character::complete::{char as nom_char, digit1, satisfy},
    combinator::recognize,
    error::ErrorKind,
    error_position,
    sequence::pair,
};

use crate::ast::{BinOp, ConcatPart, ExprAst, Span};

use super::input::{Input, span_between};
use super::util::{is_macro_name_char, is_macro_name_start, space0};

/// Maximum recursion depth accepted by [`parse_expression`]. Bounds
/// stack usage on adversarial input like `!!…!!1` or `(((…)))`. RPM
/// `%if` expressions in real specs nest only a handful of levels —
/// 128 is comfortably above what the upstream evaluator handles.
const MAX_DEPTH: u32 = 128;

/// Maximum number of atoms in a single `NumericConcat` juxtaposition.
/// Real-world specs have ≤3 atoms (`0%{?dist}.0`); 64 gives ample headroom
/// while bounding worst-case memory for adversarial input.
const MAX_CONCAT_PARTS: usize = 64;

/// Top-level entry: parse a single RPM expression. The combinator
/// stops at the first whitespace/character it can't consume — the
/// caller (see [`super::cond`]) verifies that only whitespace
/// remains before the line terminator, and falls back to `Raw` if
/// not.
pub(crate) fn parse_expression(input: Input<'_>) -> IResult<Input<'_>, ExprAst<Span>> {
    let (rest, _) = space0(input)?;
    parse_log_or(rest, 0)
}

// =====================================================================
// Generic left-associative combinator
// =====================================================================

/// Shared body for every left-associative precedence level: parse a
/// sub-expression at `next`, then greedily fold matching operators
/// into a left-leaning `Binary` tree.
fn parse_left_assoc<'a>(
    input: Input<'a>,
    depth: u32,
    ops: &[(&'static str, BinOp)],
    next: fn(Input<'a>, u32) -> IResult<Input<'a>, ExprAst<Span>>,
) -> IResult<Input<'a>, ExprAst<Span>> {
    let start = input;
    let (mut rest, mut lhs) = next(input, depth)?;
    loop {
        let (after_ws, _) = space0(rest)?;
        let matched = ops
            .iter()
            .find_map(|(sym, op)| try_op(after_ws, sym).map(|r| (*op, r)));
        match matched {
            Some((kind, after_op)) => {
                let (after_ws2, _) = space0(after_op)?;
                let (after_rhs, rhs) = next(after_ws2, depth)?;
                let span = span_between(&start, &after_rhs);
                lhs = ExprAst::Binary {
                    kind,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    data: span,
                };
                rest = after_rhs;
            }
            None => return Ok((rest, lhs)),
        }
    }
}

// =====================================================================
// Precedence levels (lowest → highest). Multi-char operators must be
// listed before any single-char prefix of them (`<=` before `<`).
// =====================================================================

fn parse_log_or(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    parse_left_assoc(input, depth, &[("||", BinOp::LogOr)], parse_log_and)
}

fn parse_log_and(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    parse_left_assoc(input, depth, &[("&&", BinOp::LogAnd)], parse_equality)
}

fn parse_equality(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    parse_left_assoc(
        input,
        depth,
        &[("==", BinOp::Eq), ("!=", BinOp::Ne)],
        parse_relational,
    )
}

fn parse_relational(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    parse_left_assoc(
        input,
        depth,
        &[
            ("<=", BinOp::Le),
            (">=", BinOp::Ge),
            ("<", BinOp::Lt),
            (">", BinOp::Gt),
        ],
        parse_unary,
    )
}

fn parse_unary(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    let depth = enter_depth(input, depth)?;
    let start = input;
    let (rest, _) = space0(input)?;
    if let Some(after_op) = try_op(rest, "!") {
        // Guard against `!=` being misread as `!` + `=...`.
        if after_op.fragment().starts_with('=') {
            return Err(nom::Err::Error(error_position!(rest, ErrorKind::Tag)));
        }
        let (after_inner, inner) = parse_unary(after_op, depth)?;
        let span = span_between(&start, &after_inner);
        return Ok((
            after_inner,
            ExprAst::Not {
                inner: Box::new(inner),
                data: span,
            },
        ));
    }
    parse_primary(rest, depth)
}

fn parse_primary(input: Input<'_>, depth: u32) -> IResult<Input<'_>, ExprAst<Span>> {
    let depth = enter_depth(input, depth)?;
    let start = input;
    let (rest, _) = space0(input)?;
    let frag = *rest.fragment();
    let first = match frag.chars().next() {
        Some(c) => c,
        None => return Err(nom::Err::Error(error_position!(rest, ErrorKind::Eof))),
    };
    match first {
        '(' => {
            let (after_open, _) = nom_char('(').parse(rest)?;
            let (after_inner, inner) = parse_log_or(after_open, depth)?;
            let (after_ws, _) = space0(after_inner)?;
            let (after_close, _) = nom_char(')').parse(after_ws)?;
            let span = span_between(&start, &after_close);
            Ok((
                after_close,
                ExprAst::Paren {
                    inner: Box::new(inner),
                    data: span,
                },
            ))
        }
        '"' => parse_string_literal(start, rest),
        '0'..='9' | '%' => parse_concat_or_single(start, rest),
        c if is_macro_name_start(c) => parse_identifier(start, rest),
        _ => Err(nom::Err::Error(error_position!(rest, ErrorKind::Char))),
    }
}

/// Parse a primary "term" that may be a juxtaposition of a digit
/// literal and one or more macro references, without intervening
/// whitespace. Examples: `0`, `%{?el8}`, `0%{?el8}`, `%{ver}0`,
/// `1%{?dist}0`.
///
/// `start` is the cursor at the head of the primary (before any
/// `space0` skip); `rest` is the cursor positioned at the first
/// non-whitespace character.
///
/// Returns a single [`ExprAst::Integer`]/[`ExprAst::Macro`] when only
/// one atom is consumed, or [`ExprAst::NumericConcat`] when two or more
/// adjacent atoms are joined.
fn parse_concat_or_single<'a>(
    start: Input<'a>,
    rest: Input<'a>,
) -> IResult<Input<'a>, ExprAst<Span>> {
    let term_start = rest;
    // Typical NumericConcat has 2-3 parts (`0%{?dist}` / `0%{?dist}.0`);
    // pre-allocate the common capacity.
    let mut parts: Vec<ConcatPart<Span>> = Vec::with_capacity(2);
    let mut cursor = rest;
    loop {
        if parts.len() >= MAX_CONCAT_PARTS {
            break;
        }
        let before_atom = cursor;
        // ASCII fast path: we only branch on the digit range and `%`,
        // both single-byte ASCII, so inspecting the leading byte is
        // sufficient and avoids the UTF-8 decode cost of `chars()` on
        // every loop iteration.
        let next_ch = match cursor.fragment().as_bytes().first().copied() {
            Some(b) => b,
            None => break,
        };
        let (after_atom, part) = match next_ch {
            b'0'..=b'9' => parse_concat_literal(cursor)?,
            b'%' => parse_concat_macro(cursor)?,
            _ => break,
        };
        debug_assert!(
            after_atom.location_offset() > before_atom.location_offset(),
            "atom parser must make progress (digit1/% guarantees ≥1 byte)"
        );
        parts.push(part);
        cursor = after_atom;
    }
    debug_assert!(
        !parts.is_empty(),
        "parse_primary dispatches here only for '0'..='9' | '%'; both consume ≥1 byte"
    );
    if parts.len() == 1 {
        // Single atom — unwrap into the canonical Integer/Macro variant
        // so existing consumers don't need to learn about NumericConcat
        // for the common `42` / `%{?rhel}` case.
        let span = span_between(&start, &cursor);
        let only = match parts.pop() {
            Some(p) => p,
            None => unreachable!("parts.len() == 1 checked above"),
        };
        return match unwrap_single_part(only, span) {
            Some(ast) => Ok((cursor, ast)),
            // i64 overflow on a bare literal — preserve the previous
            // `parse_integer` behaviour: fail so cond.rs falls back to
            // `CondExpr::Raw`.
            None => Err(nom::Err::Error(error_position!(rest, ErrorKind::Digit))),
        };
    }
    let span = span_between(&term_start, &cursor);
    Ok((cursor, ExprAst::NumericConcat { parts, data: span }))
}

/// Parse one literal-digit part of a juxtaposition. Consumes `digit1`.
/// Returns a [`ConcatPart::Literal`] whose `text` is the digit string
/// verbatim (no `i64` parsing — the value is decided after the whole
/// concat is materialised and macros expanded).
fn parse_concat_literal<'a>(rest: Input<'a>) -> IResult<Input<'a>, ConcatPart<Span>> {
    let start = rest;
    let (after_digits, digits) = digit1(rest)?;
    let span = span_between(&start, &after_digits);
    Ok((
        after_digits,
        ConcatPart::Literal {
            text: digits.fragment().to_string(),
            data: span,
        },
    ))
}

/// Parse one macro reference part of a juxtaposition. Uses
/// [`find_brace_close`] for `%{…}` brace-balanced bodies and the
/// `is_macro_name_*` predicates for bare-name macros.
fn parse_concat_macro<'a>(rest: Input<'a>) -> IResult<Input<'a>, ConcatPart<Span>> {
    let start = rest;
    let frag = *rest.fragment();
    let bytes = frag.as_bytes();
    debug_assert!(bytes.first() == Some(&b'%'));
    let body_start = 1usize;
    let end_offset = match bytes.get(body_start) {
        Some(b'{') => find_brace_close(frag, body_start)
            .ok_or_else(|| nom::Err::Error(error_position!(rest, ErrorKind::Char)))?,
        Some(c) if is_macro_name_start(*c as char) => {
            body_start
                + frag[body_start..]
                    .find(|c: char| !is_macro_name_char(c))
                    .unwrap_or(frag.len() - body_start)
        }
        _ => return Err(nom::Err::Error(error_position!(rest, ErrorKind::Char))),
    };
    let macro_text = frag[..end_offset].to_owned();
    let (after_macro, _) = nom::Input::take_split(&rest, end_offset);
    let span = span_between(&start, &after_macro);
    Ok((
        after_macro,
        ConcatPart::Macro {
            text: macro_text,
            data: span,
        },
    ))
}

/// Single-atom shortcut: build the canonical [`ExprAst::Integer`] /
/// [`ExprAst::Macro`] node instead of wrapping in a length-1
/// [`ExprAst::NumericConcat`]. Returns `None` when the part is a
/// literal that overflows `i64` so the caller fails the parse and
/// `cond.rs` falls back to [`crate::ast::CondExpr::Raw`] (matches the
/// original `parse_integer` behaviour).
///
/// **Lossy:** `i64::from_str` failure does not distinguish overflow
/// from malformed digit-strings here; both collapse into `None`.
/// Threading `ParserState` to emit `W_IF_INT_OVERFLOW` specifically
/// is tracked by the TODO in this function's body.
fn unwrap_single_part(part: ConcatPart<Span>, outer_span: Span) -> Option<ExprAst<Span>> {
    match part {
        ConcatPart::Literal { text, .. } => {
            // TODO(diag): emit W_IF_INT_OVERFLOW for overflow specifically.
            // Requires threading `ParserState` into this helper; until then
            // the `Option` return path collapses overflow and InvalidDigit
            // indistinguishably.
            let value: i64 = text.parse().ok()?;
            Some(ExprAst::Integer {
                value,
                data: outer_span,
            })
        }
        ConcatPart::Macro { text, .. } => Some(ExprAst::Macro {
            text,
            data: outer_span,
        }),
    }
}

// =====================================================================
// Operand parsers
// =====================================================================

fn parse_string_literal<'a>(
    start: Input<'a>,
    rest: Input<'a>,
) -> IResult<Input<'a>, ExprAst<Span>> {
    let (after_open, _) = nom_char('"').parse(rest)?;
    // RPM string literals are line-bounded. Reject embedded `\n`/`\r`
    // so a stray unclosed quote can't drag subsequent spec lines into
    // a single literal.
    let (after_body, body) =
        nom::bytes::complete::take_while(|c: char| c != '"' && c != '\n' && c != '\r')(after_open)?;
    let (after_close, _) = nom_char('"').parse(after_body)?;
    let span = span_between(&start, &after_close);
    Ok((
        after_close,
        ExprAst::String {
            value: body.fragment().to_string(),
            data: span,
        },
    ))
}

/// Given `frag` whose byte at `open_pos` is `{`, return the byte
/// offset *just past* the matching `}`, tracking nested `{`/`}` so
/// `%{?a:%{?b}}` is captured as one extent. Returns `None` if no
/// closing brace balances out at the same depth.
///
/// Not string-literal-aware: `%{lua:foo("}")}` will close at the
/// inner `}` and the surrounding parse falls back to `Raw`.
fn find_brace_close(frag: &str, open_pos: usize) -> Option<usize> {
    let bytes = frag.as_bytes();
    debug_assert!(bytes.get(open_pos) == Some(&b'{'));
    let mut depth: u32 = 0;
    let mut i = open_pos;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_identifier<'a>(start: Input<'a>, rest: Input<'a>) -> IResult<Input<'a>, ExprAst<Span>> {
    let (after_id, ident) = recognize(pair(
        satisfy(is_macro_name_start),
        take_while1(is_macro_name_char),
    ))
    .parse(rest)
    .or_else(|_: nom::Err<nom::error::Error<Input<'_>>>| {
        // Single-char identifier (e.g. just `a`).
        recognize(satisfy(is_macro_name_start)).parse(rest)
    })?;
    let span = span_between(&start, &after_id);
    Ok((
        after_id,
        ExprAst::Identifier {
            name: ident.fragment().to_string(),
            data: span,
        },
    ))
}

// =====================================================================
// Helpers
// =====================================================================

/// Try to consume `op` at the current position. Returns the
/// post-consumption input on success, `None` otherwise. Does **not**
/// consume leading whitespace.
fn try_op<'a>(input: Input<'a>, op: &'static str) -> Option<Input<'a>> {
    let frag = *input.fragment();
    frag.starts_with(op)
        .then(|| nom::Input::take_split(&input, op.len()).0)
}

/// Bump the recursion depth and bail out if it exceeds [`MAX_DEPTH`].
fn enter_depth(
    input: Input<'_>,
    depth: u32,
) -> Result<u32, nom::Err<nom::error::Error<Input<'_>>>> {
    if depth >= MAX_DEPTH {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::TooLarge)));
    }
    Ok(depth + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_full(src: &str) -> Option<ExprAst<Span>> {
        let input = Input::new(src);
        let (rest, ast) = parse_expression(input).ok()?;
        let (rest, _) = space0(rest).ok()?;
        if rest.fragment().is_empty() {
            Some(ast)
        } else {
            None
        }
    }

    #[test]
    fn integer_literal() {
        let ast = parse_full("42").unwrap();
        assert!(matches!(ast, ExprAst::Integer { value: 42, .. }));
    }

    #[test]
    fn logical_and() {
        let ast = parse_full("1 && 0").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogAnd,
                ..
            } => {}
            other => panic!("expected LogAnd, got {other:?}"),
        }
    }

    #[test]
    fn precedence_and_over_or() {
        // `1 || 0 && 1` should parse as `1 || (0 && 1)`.
        let ast = parse_full("1 || 0 && 1").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogOr,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, ExprAst::Integer { value: 1, .. }));
                assert!(matches!(
                    *rhs,
                    ExprAst::Binary {
                        kind: BinOp::LogAnd,
                        ..
                    }
                ));
            }
            other => panic!("expected LogOr at root, got {other:?}"),
        }
    }

    #[test]
    fn string_equality() {
        let ast = parse_full("\"foo\" == \"bar\"").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::Eq,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, ExprAst::String { ref value, .. } if value == "foo"));
                assert!(matches!(*rhs, ExprAst::String { ref value, .. } if value == "bar"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn relational_and_logical() {
        // `0%{?rhel} >= 8 && 0%{?rhel} < 10`
        let ast = parse_full("0 >= 8 && 0 < 10").unwrap();
        assert!(matches!(
            ast,
            ExprAst::Binary {
                kind: BinOp::LogAnd,
                ..
            }
        ));
    }

    #[test]
    fn negation() {
        let ast = parse_full("!1").unwrap();
        assert!(matches!(ast, ExprAst::Not { .. }));
    }

    #[test]
    fn parens() {
        let ast = parse_full("(1 || 0)").unwrap();
        assert!(matches!(ast, ExprAst::Paren { .. }));
    }

    #[test]
    fn macro_braced() {
        let ast = parse_full("%{?rhel}").unwrap();
        match ast {
            ExprAst::Macro { text, .. } => {
                assert_eq!(text, "%{?rhel}");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn macro_in_comparison() {
        let ast = parse_full("%{?rhel} >= 8").unwrap();
        assert!(matches!(
            ast,
            ExprAst::Binary {
                kind: BinOp::Ge,
                ..
            }
        ));
    }

    #[test]
    fn macro_nested_braces() {
        // `%{?a:%{?b}}` should be captured as a single Macro extent.
        let ast = parse_full("%{?a:%{?b}}").unwrap();
        match ast {
            ExprAst::Macro { text, .. } => {
                assert_eq!(text, "%{?a:%{?b}}");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn rejects_unclosed_brace() {
        // No matching `}` — falls back via parse failure.
        assert!(parse_full("%{?a:%{?b}").is_none());
    }

    #[test]
    fn double_negation() {
        let ast = parse_full("!!1").unwrap();
        match ast {
            ExprAst::Not { inner, .. } => {
                assert!(matches!(*inner, ExprAst::Not { .. }));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn ne_operator() {
        let ast = parse_full("1 != 0").unwrap();
        assert!(matches!(
            ast,
            ExprAst::Binary {
                kind: BinOp::Ne,
                ..
            }
        ));
    }

    #[test]
    fn le_and_ge_operators() {
        assert!(matches!(
            parse_full("1 <= 2"),
            Some(ExprAst::Binary {
                kind: BinOp::Le,
                ..
            })
        ));
        assert!(matches!(
            parse_full("3 >= 2"),
            Some(ExprAst::Binary {
                kind: BinOp::Ge,
                ..
            })
        ));
    }

    #[test]
    fn multi_or_chain_is_left_assoc() {
        // `1 || 0 || 1` => `(1 || 0) || 1`
        let ast = parse_full("1 || 0 || 1").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogOr,
                lhs,
                ..
            } => {
                assert!(matches!(
                    *lhs,
                    ExprAst::Binary {
                        kind: BinOp::LogOr,
                        ..
                    }
                ));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn mixed_precedence() {
        // `1 < 2 && 3 == 4 || 5 != 6`
        //  → ((1<2) && (3==4)) || (5!=6)
        let ast = parse_full("1 < 2 && 3 == 4 || 5 != 6").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogOr,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(
                    *lhs,
                    ExprAst::Binary {
                        kind: BinOp::LogAnd,
                        ..
                    }
                ));
                assert!(matches!(
                    *rhs,
                    ExprAst::Binary {
                        kind: BinOp::Ne,
                        ..
                    }
                ));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn bare_identifier() {
        let ast = parse_full("foo").unwrap();
        match ast {
            ExprAst::Identifier { name, .. } => assert_eq!(name, "foo"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn deeply_nested_parens() {
        let ast = parse_full("((((1))))").unwrap();
        // Should peel to Integer(1).
        assert!(matches!(
            ast.peel_parens(),
            ExprAst::Integer { value: 1, .. }
        ));
    }

    #[test]
    fn rejects_arithmetic() {
        // `+` is not in the modelled grammar — must fail.
        assert!(parse_full("1 + 2").is_none());
    }

    #[test]
    fn rejects_integer_overflow() {
        // 21 nines exceeds i64::MAX — must fail, letting cond.rs fall
        // back to Raw.
        assert!(parse_full("999999999999999999999").is_none());
    }

    #[test]
    fn rejects_string_with_newline() {
        // Embedded `\n` is not a valid RPM string literal byte.
        assert!(parse_full("\"foo\nbar\"").is_none());
    }

    #[test]
    fn rejects_trailing_garbage() {
        // After `1 || 0`, an unexpected `xyz` (with no operator)
        // means the grammar didn't consume everything; the helper
        // returns None.
        assert!(parse_full("1 || 0 xyz").is_none());
    }

    #[test]
    fn rejects_depth_overflow() {
        // 200 levels of `!` exceeds MAX_DEPTH (128).
        let src: String = "!".repeat(200) + "1";
        assert!(parse_full(&src).is_none());
    }

    #[test]
    fn stops_at_newline() {
        // `parse_expression` itself must not consume past a newline.
        let input = Input::new("1 || 0\nfoo");
        let (rest, _ast) = parse_expression(input).unwrap();
        assert!(rest.fragment().starts_with('\n'));
    }

    #[test]
    fn spans_cover_full_subexpression() {
        let input = Input::new("1 || 0");
        let (_rest, ast) = parse_expression(input).unwrap();
        let span = ast.data();
        assert_eq!(span.start_byte, 0);
        assert_eq!(span.end_byte, 6);
    }

    // -----------------------------------------------------------------
    // NumericConcat — `0%{?el8}` idiom and friends
    // -----------------------------------------------------------------

    #[test]
    fn parses_zero_prefix_macro_as_numeric_concat() {
        let ast = parse_full("0%{?el8}").unwrap();
        match ast {
            ExprAst::NumericConcat { parts, .. } => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(
                    &parts[0],
                    ConcatPart::Literal { text, .. } if text == "0"
                ));
                assert!(matches!(
                    &parts[1],
                    ConcatPart::Macro { text, .. } if text == "%{?el8}"
                ));
            }
            other => panic!("expected NumericConcat, got {other:?}"),
        }
    }

    #[test]
    fn parses_macro_suffix_literal_as_numeric_concat() {
        let ast = parse_full("%{ver}0").unwrap();
        match ast {
            ExprAst::NumericConcat { parts, .. } => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(
                    &parts[0],
                    ConcatPart::Macro { text, .. } if text == "%{ver}"
                ));
                assert!(matches!(
                    &parts[1],
                    ConcatPart::Literal { text, .. } if text == "0"
                ));
            }
            other => panic!("expected NumericConcat, got {other:?}"),
        }
    }

    #[test]
    fn parses_chained_or_with_numeric_concats() {
        // `0%{?el8} || 0%{?el9} || 0%{?el10}` — three NumericConcats
        // chained by `||`, left-associative tree.
        let ast = parse_full("0%{?el8} || 0%{?el9} || 0%{?el10}").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogOr,
                lhs,
                rhs,
                ..
            } => {
                // rhs is the rightmost NumericConcat.
                assert!(matches!(*rhs, ExprAst::NumericConcat { .. }));
                // lhs is the left-leaning sub-tree.
                match *lhs {
                    ExprAst::Binary {
                        kind: BinOp::LogOr,
                        lhs: inner_lhs,
                        rhs: inner_rhs,
                        ..
                    } => {
                        assert!(matches!(*inner_lhs, ExprAst::NumericConcat { .. }));
                        assert!(matches!(*inner_rhs, ExprAst::NumericConcat { .. }));
                    }
                    other => panic!("expected nested LogOr, got {other:?}"),
                }
            }
            other => panic!("expected LogOr at root, got {other:?}"),
        }
    }

    #[test]
    fn parses_relational_with_numeric_concat() {
        // `0%{?redos_version} >= 800` — NumericConcat compared to literal.
        let ast = parse_full("0%{?redos_version} >= 800").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::Ge,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, ExprAst::NumericConcat { .. }));
                assert!(matches!(*rhs, ExprAst::Integer { value: 800, .. }));
            }
            other => panic!("expected Ge, got {other:?}"),
        }
    }

    #[test]
    fn parses_full_user_expression() {
        // The motivating real-world condition from postgrespro.centos.spec.
        // Expected tree (precedence: `>=` over `||`, `||` left-assoc):
        //   ((NC || NC) || NC) || (NC >= 800)
        let ast =
            parse_full("0%{?el8} || 0%{?el9} || 0%{?el10} || 0%{?redos_version} >= 800").unwrap();
        match ast {
            ExprAst::Binary {
                kind: BinOp::LogOr,
                rhs,
                ..
            } => {
                // The rightmost operand of the top-level `||` must be
                // the `>=` comparison (binds tighter than `||`).
                assert!(matches!(
                    *rhs,
                    ExprAst::Binary {
                        kind: BinOp::Ge,
                        ..
                    }
                ));
            }
            other => panic!("expected LogOr at root, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_breaks_concat_into_trailing_garbage() {
        // `0 %{?el8}` — with whitespace between the digit and macro,
        // `parse_full` consumes `0` as Integer, then `%{?el8}` is left
        // over and `parse_full`'s "rest must be empty" check fails.
        assert!(parse_full("0 %{?el8}").is_none());
    }

    #[test]
    fn three_part_concat_macro_literal_macro() {
        // `%{a}1%{b}` — exotic but legal: macro, literal `1`, macro.
        let ast = parse_full("%{a}1%{b}").unwrap();
        match ast {
            ExprAst::NumericConcat { parts, .. } => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], ConcatPart::Macro { text, .. } if text == "%{a}"));
                assert!(matches!(&parts[1], ConcatPart::Literal { text, .. } if text == "1"));
                assert!(matches!(&parts[2], ConcatPart::Macro { text, .. } if text == "%{b}"));
            }
            other => panic!("expected NumericConcat, got {other:?}"),
        }
    }
}
