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

use crate::ast::{BinOp, ExprAst, Span};

use super::input::{Input, span_between};
use super::util::{is_macro_name_char, is_macro_name_start, space0};

/// Maximum recursion depth accepted by [`parse_expression`]. Bounds
/// stack usage on adversarial input like `!!…!!1` or `(((…)))`. RPM
/// `%if` expressions in real specs nest only a handful of levels —
/// 128 is comfortably above what the upstream evaluator handles.
const MAX_DEPTH: u32 = 128;

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
        '0'..='9' => parse_integer(start, rest),
        '%' => parse_macro_primary(start, rest),
        c if is_macro_name_start(c) => parse_identifier(start, rest),
        _ => Err(nom::Err::Error(error_position!(rest, ErrorKind::Char))),
    }
}

// =====================================================================
// Operand parsers
// =====================================================================

fn parse_integer<'a>(start: Input<'a>, rest: Input<'a>) -> IResult<Input<'a>, ExprAst<Span>> {
    let (after_digits, digits) = digit1(rest)?;
    // i64 overflow falls back to a parse error; the caller (cond.rs)
    // then drops to `CondExpr::Raw`, preserving the source verbatim.
    // TODO(diag): emit W_IF_INT_OVERFLOW for overflow specifically.
    // Distinguishing overflow from malformed digits requires threading
    // `ParserState` through this layer; tracked as a follow-up.
    let value: i64 = digits
        .fragment()
        .parse()
        .map_err(|_| nom::Err::Error(error_position!(rest, ErrorKind::Digit)))?;
    let span = span_between(&start, &after_digits);
    Ok((after_digits, ExprAst::Integer { value, data: span }))
}

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

fn parse_macro_primary<'a>(start: Input<'a>, rest: Input<'a>) -> IResult<Input<'a>, ExprAst<Span>> {
    // A `%` token in an expression is a macro reference. Forms:
    //   - `%{...}` / `%{?...}`: braced, with brace-depth tracking so
    //     nested `%{?a:%{?b}}` is captured as one extent.
    //   - `%name`:               bare.
    // The whole verbatim slice (including the leading `%` and braces)
    // is stored on `ExprAst::Macro` so the printer can emit it as-is.
    // Only `%{…}` braced and bare `%name` are recognised; `%(shell)`
    // and `%[expr]` macros are unsupported in `%if` expressions and
    // the surrounding parse falls back to `Raw`.
    // Stored as a flat String — callers that need structured walking
    // should invoke `parse_macro_ref` directly on the source slice.
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
        ExprAst::Macro {
            text: macro_text,
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
}
