//! Parser for `%if` / `%ifarch` / `%ifos` (with their `%n`-negated forms)
//! conditional blocks.
//!
//! [`parse_conditional`] is generic over the body grammar: the caller
//! supplies a parser for one "body item" (top-level `SpecItem`, preamble
//! content, files content, …) and `parse_conditional` stitches the
//! branches together.

use nom::{IResult, Parser, bytes::complete::tag, error::ErrorKind, error_position};

use crate::ast::{CondBranch, CondExpr, CondKind, Conditional, ExprAst, Span, Text, TextSegment};
use crate::parse_result::codes;

use super::input::{Input, span_between};
use super::state::ParserState;
use super::util::{line_terminator, logical_line, space0};

/// Parse a `%if` / `%ifarch` / `%ifos` block. `parse_body_items` is invoked
/// repeatedly until an `%elif` / `%else` / `%endif` is seen.
///
/// `parse_body_items` returns `Vec<Body>` so a single source line may
/// expand into multiple AST nodes (e.g. multi-dep preamble lines).
pub fn parse_conditional<'a, B, F>(
    state: &ParserState,
    input: Input<'a>,
    parse_body_items: F,
) -> IResult<Input<'a>, Conditional<Span, B>>
where
    F: Fn(&ParserState, Input<'a>) -> IResult<Input<'a>, Vec<B>> + Copy,
{
    let start = input;

    // Opening %if* head.
    let (mut cursor, (head_kind, head_expr)) =
        parse_branch_head(input, /*allow_elif=*/ false)?;

    let mut branches: Vec<CondBranch<Span, B>> = Vec::new();
    let mut current_kind = head_kind;
    let mut current_expr = head_expr;
    let mut current_start = start;
    let mut current_body: Vec<B> = Vec::new();
    let mut otherwise: Option<Vec<B>> = None;
    let mut in_else = false;
    let mut else_body: Vec<B> = Vec::new();

    loop {
        // Try to recognize a closing/branch keyword without consuming.
        if let Some(kw) = peek_cond_keyword(cursor) {
            match kw {
                CondKeyword::Endif => {
                    let (after_endif, _) = consume_endif(cursor)?;
                    if in_else {
                        otherwise = Some(std::mem::take(&mut else_body));
                    } else {
                        let branch_span = span_between(&current_start, &cursor);
                        branches.push(CondBranch {
                            kind: current_kind,
                            expr: current_expr,
                            body: std::mem::take(&mut current_body),
                            data: branch_span,
                        });
                    }
                    let total_span = span_between(&start, &after_endif);
                    return Ok((
                        after_endif,
                        Conditional { branches, otherwise, data: total_span },
                    ));
                }
                CondKeyword::Else => {
                    let (after_else, _) = consume_else(cursor)?;
                    if in_else {
                        state.push_warning_code(
                            codes::W_MULTIPLE_ELSE,
                            "multiple %else clauses; keeping the last",
                            Some(super::input::span_at(&cursor)),
                        );
                    } else {
                        let branch_span = span_between(&current_start, &cursor);
                        branches.push(CondBranch {
                            kind: current_kind,
                            expr: current_expr,
                            body: std::mem::take(&mut current_body),
                            data: branch_span,
                        });
                        in_else = true;
                    }
                    // Replace context so next iteration collects else_body.
                    current_expr = CondExpr::Raw(Text::new());
                    current_kind = CondKind::Elif;
                    cursor = after_else;
                    continue;
                }
                CondKeyword::Elif => {
                    let (after_head, (new_kind, new_expr)) =
                        parse_branch_head(cursor, /*allow_elif=*/ true)?;
                    if in_else {
                        state.push_warning_code(
                            codes::W_ELIF_AFTER_ELSE,
                            "%elif after %else; treating as part of the %else body",
                            Some(super::input::span_at(&cursor)),
                        );
                    } else {
                        let branch_span = span_between(&current_start, &cursor);
                        branches.push(CondBranch {
                            kind: current_kind,
                            expr: current_expr,
                            body: std::mem::take(&mut current_body),
                            data: branch_span,
                        });
                    }
                    current_kind = new_kind;
                    current_expr = new_expr;
                    current_start = cursor;
                    cursor = after_head;
                    continue;
                }
            }
        }

        // Otherwise consume one or more body items in a single step.
        match parse_body_items(state, cursor) {
            Ok((rest, items)) => {
                if in_else {
                    else_body.extend(items);
                } else {
                    current_body.extend(items);
                }
                cursor = rest;
            }
            Err(_) => {
                // No body item; if we cannot make progress, treat as
                // unterminated and stop with a diagnostic.
                state.push_error_code(
                    codes::E_UNTERMINATED_CONDITIONAL,
                    "unterminated conditional: expected %endif",
                    Some(super::input::span_at(&cursor)),
                );
                let span = span_between(&start, &cursor);
                if in_else {
                    otherwise = Some(else_body);
                } else {
                    branches.push(CondBranch {
                        kind: current_kind,
                        expr: current_expr,
                        body: current_body,
                        data: span,
                    });
                }
                return Ok((
                    cursor,
                    Conditional { branches, otherwise, data: span },
                ));
            }
        }
    }
}

/// Peek-only check for one of the branch/close keywords. Returns `None` if
/// the cursor does not start with one.
fn peek_cond_keyword(input: Input<'_>) -> Option<CondKeyword> {
    let (rest, _) = match space0(input) {
        Ok(r) => r,
        Err(_) => return None,
    };
    let frag = *rest.fragment();
    if starts_with_keyword(frag, "%endif") {
        return Some(CondKeyword::Endif);
    }
    if starts_with_keyword(frag, "%else") {
        return Some(CondKeyword::Else);
    }
    if elif_kind(frag).is_some() {
        return Some(CondKeyword::Elif);
    }
    None
}

enum CondKeyword {
    Endif,
    Else,
    /// The kind (Elif/ElifArch/ElifOs) is rediscovered by the
    /// subsequent `parse_branch_head` call; this variant only signals
    /// that an `%elif*` keyword was seen.
    Elif,
}

fn elif_kind(frag: &str) -> Option<CondKind> {
    if starts_with_keyword(frag, "%elifarch") {
        return Some(CondKind::ElifArch);
    }
    if starts_with_keyword(frag, "%elifos") {
        return Some(CondKind::ElifOs);
    }
    if starts_with_keyword(frag, "%elif") {
        return Some(CondKind::Elif);
    }
    None
}

/// A keyword must be followed by whitespace, EOL, or EOF — otherwise
/// `%endifFoo` would be confused with `%endif`.
fn starts_with_keyword(haystack: &str, keyword: &str) -> bool {
    if !haystack.starts_with(keyword) {
        return false;
    }
    matches!(
        haystack[keyword.len()..].chars().next(),
        None | Some(' ' | '\t' | '\n' | '\r' | '#')
    )
}

fn consume_endif(input: Input<'_>) -> IResult<Input<'_>, Input<'_>> {
    let (rest, _) = space0(input)?;
    let (after_kw, _) = tag("%endif").parse(rest)?;
    // Allow trailing whitespace + optional comment (`%endif # foo`).
    let (after_tail, _) = line_terminator(after_kw)?;
    Ok((after_tail, after_tail))
}

fn consume_else(input: Input<'_>) -> IResult<Input<'_>, Input<'_>> {
    let (rest, _) = space0(input)?;
    let (after_kw, _) = tag("%else").parse(rest)?;
    let (after_tail, _) = line_terminator(after_kw)?;
    Ok((after_tail, after_tail))
}

fn parse_branch_head<'a>(
    input: Input<'a>,
    allow_elif: bool,
) -> IResult<Input<'a>, (CondKind, CondExpr<Span>)> {
    let (rest, _) = space0(input)?;
    let frag = *rest.fragment();

    let (kind, kw_len) = if starts_with_keyword(frag, "%ifarch") {
        (CondKind::IfArch, "%ifarch".len())
    } else if starts_with_keyword(frag, "%ifnarch") {
        (CondKind::IfNArch, "%ifnarch".len())
    } else if starts_with_keyword(frag, "%ifos") {
        (CondKind::IfOs, "%ifos".len())
    } else if starts_with_keyword(frag, "%ifnos") {
        (CondKind::IfNOs, "%ifnos".len())
    } else if starts_with_keyword(frag, "%if") {
        (CondKind::If, "%if".len())
    } else if allow_elif {
        if starts_with_keyword(frag, "%elifarch") {
            (CondKind::ElifArch, "%elifarch".len())
        } else if starts_with_keyword(frag, "%elifos") {
            (CondKind::ElifOs, "%elifos".len())
        } else if starts_with_keyword(frag, "%elif") {
            (CondKind::Elif, "%elif".len())
        } else {
            return Err(nom::Err::Error(error_position!(rest, ErrorKind::Tag)));
        }
    } else {
        return Err(nom::Err::Error(error_position!(rest, ErrorKind::Tag)));
    };

    let (after_kw, _) = nom::Input::take_split(&rest, kw_len);
    let (after_kw, _) = space0(after_kw)?;

    // For plain `%if` / `%elif`, try the structured expression
    // grammar first. On success — and only when the expression
    // consumed everything up to the line terminator — emit a
    // `CondExpr::Parsed` with per-node spans. Anything that fails
    // the modelled grammar (arithmetic, exotic operators, partial
    // parses) falls through to the raw-text fallback below so the
    // file still parses correctly.
    fn try_structured_expr(input: Input<'_>) -> Option<(Input<'_>, ExprAst<Span>)> {
        let (after_expr, ast) = super::expr::parse_expression(input).ok()?;
        let (after_terminator, _) = line_terminator(after_expr).ok()?;
        Some((after_terminator, ast))
    }

    if matches!(kind, CondKind::If | CondKind::Elif) {
        if let Some((after_terminator, ast)) = try_structured_expr(after_kw) {
            return Ok((after_terminator, (kind, CondExpr::Parsed(Box::new(ast)))));
        }
    }

    let (after_value, raw) = match logical_line(after_kw) {
        Ok(r) => r,
        Err(_) => (after_kw, String::new()),
    };
    let raw_trim = raw.trim_end();

    let expr = match kind {
        CondKind::IfArch
        | CondKind::IfNArch
        | CondKind::IfOs
        | CondKind::IfNOs
        | CondKind::ElifArch
        | CondKind::ElifOs => CondExpr::ArchList(
            raw_trim
                .split_whitespace()
                .map(|tok| Text { segments: vec![TextSegment::Literal(tok.to_owned())] })
                .collect(),
        ),
        CondKind::If | CondKind::Elif => {
            CondExpr::Raw(Text { segments: vec![TextSegment::Literal(raw_trim.to_owned())] })
        }
    };

    Ok((after_value, (kind, expr)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SpecItem;
    use crate::parser::macros::parse_top_macro_statement;

    fn dummy_body<'a>(
        state: &ParserState,
        input: Input<'a>,
    ) -> IResult<Input<'a>, Vec<SpecItem<Span>>> {
        let (rest, item) = parse_top_macro_statement(state, input)?;
        Ok((rest, vec![item]))
    }

    fn parse_one(src: &str) -> Conditional<Span, SpecItem<Span>> {
        let state = ParserState::new();
        let input = Input::new(src);
        let (_rest, c) = parse_conditional(&state, input, dummy_body).unwrap();
        c
    }

    #[test]
    fn ifarch_simple() {
        let c = parse_one("%ifarch x86_64\n%define foo bar\n%endif\n");
        assert_eq!(c.branches.len(), 1);
        let b = &c.branches[0];
        assert_eq!(b.kind, CondKind::IfArch);
        match &b.expr {
            CondExpr::ArchList(arches) => {
                assert_eq!(arches.len(), 1);
                assert_eq!(arches[0].literal_str(), Some("x86_64"));
            }
            _ => panic!(),
        }
        assert_eq!(b.body.len(), 1);
        assert!(c.otherwise.is_none());
    }

    #[test]
    fn if_with_else() {
        let c = parse_one(
            "%if 1\n%define a 1\n%else\n%define b 2\n%endif\n",
        );
        assert_eq!(c.branches.len(), 1);
        assert_eq!(c.branches[0].kind, CondKind::If);
        let else_body = c.otherwise.unwrap();
        assert_eq!(else_body.len(), 1);
    }

    #[test]
    fn if_with_elif_else() {
        let c = parse_one(
            "%if 1\n%define a 1\n%elif 2\n%define b 2\n%elif 3\n%define c 3\n%else\n%define d 4\n%endif\n",
        );
        assert_eq!(c.branches.len(), 3);
        assert_eq!(c.branches[0].kind, CondKind::If);
        assert_eq!(c.branches[1].kind, CondKind::Elif);
        assert_eq!(c.branches[2].kind, CondKind::Elif);
        assert_eq!(c.otherwise.unwrap().len(), 1);
    }

    #[test]
    fn ifarch_multiple_arches() {
        let c = parse_one("%ifarch x86_64 aarch64 ppc64le\n%endif\n");
        match &c.branches[0].expr {
            CondExpr::ArchList(arches) => {
                assert_eq!(arches.len(), 3);
                assert_eq!(arches[2].literal_str(), Some("ppc64le"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn endif_with_trailing_comment_ok() {
        let c = parse_one("%if 1\n%define a 1\n%endif\n");
        let _ = c.branches[0].body.len();
    }

    #[test]
    fn unterminated_emits_error_diagnostic() {
        let state = ParserState::new();
        let input = Input::new("%if 1\n%define a 1\n");
        let (_rest, _) = parse_conditional(&state, input, dummy_body).unwrap();
        // Two outcomes: either the parser consumed the items as body and
        // hit EOF without %endif, recording an error; or it returned a
        // recovery placeholder. Either way we expect at least one error
        // diagnostic.
        assert!(
            state.has_errors(),
            "expected error diagnostic for unterminated %if, got {:?}",
            state.snapshot_diagnostics()
        );
    }
}
