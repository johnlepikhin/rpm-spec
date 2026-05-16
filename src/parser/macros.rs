//! Parsers for top-level macro statements:
//!
//! - `%define NAME [(opts)] BODY`
//! - `%global NAME [(opts)] BODY`
//! - `%undefine NAME`
//! - `%bcond NAME DEFAULT`
//! - `%bcond_with NAME`
//! - `%bcond_without NAME`
//! - `%include PATH`
//! - `%dnl ...` comments
//! - `#` comments
//! - bare top-level macro calls (`%dump`, `%trace`, `%lua{...}` — anything
//!   else that parses as a single [`crate::ast::MacroRef`])
//!
//! Each parser advances past the trailing newline (or EOF) so the
//! top-level loop can call them in sequence.

use nom::{IResult, Parser, bytes::complete::tag, error::ErrorKind, error_position};

use crate::ast::{
    BuildCondStyle, BuildCondition, Comment, CommentStyle, IncludeDirective, MacroDef,
    MacroDefKind, Span, SpecItem, Text,
};

use super::input::{Input, span_between};
use super::state::ParserState;
use super::text::{parse_body_as_text, parse_macro_ref};
use super::util::{is_macro_name_char, is_macro_name_start, line_terminator, logical_line, space0};

/// Try to recognize one of the keyword-led top-level macro statements at
/// the current cursor. Returns `Ok((rest, SpecItem))` if a statement was
/// parsed, propagates a nom error otherwise.
pub fn parse_top_macro_statement<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    // Allow optional leading whitespace (rpm ≥ 4.20).
    let (input, _) = space0(input)?;

    // The statement must start with '%'.
    if !input.fragment().starts_with('%') {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }

    // Try keyword-led forms first, in order. Order matters: %bcond_with
    // must be tried before %bcond, etc.
    if let Ok(r) = parse_define(state, input, "%global", MacroDefKind::Global) {
        return Ok(r);
    }
    if let Ok(r) = parse_define(state, input, "%define", MacroDefKind::Define) {
        return Ok(r);
    }
    if let Ok(r) = parse_undefine(state, input) {
        return Ok(r);
    }
    if let Ok(r) = parse_bcond_with(state, input) {
        return Ok(r);
    }
    if let Ok(r) = parse_bcond_without(state, input) {
        return Ok(r);
    }
    if let Ok(r) = parse_bcond(state, input) {
        return Ok(r);
    }
    if let Ok(r) = parse_include(state, input) {
        return Ok(r);
    }
    if let Ok(r) = parse_dnl(state, input) {
        return Ok(r);
    }

    // Not a keyword-led statement.
    Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)))
}

fn parse_define<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &'static str,
    kind: MacroDefKind,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag(keyword).parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;

    // NAME
    let (after_name, name) = take_macro_name(after_kw)?;

    // optional (opts)
    let (after_opts, opts) = take_optional_opts(after_name);
    let (after_opts, _) = space0(after_opts)?;

    // body via logical_line (handles trailing `\` continuations).
    let (after_body, body_raw) = match logical_line(after_opts) {
        Ok(r) => r,
        Err(_) => (after_opts, String::new()),
    };

    let body = parse_body_as_text(state, &body_raw);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        SpecItem::MacroDef(MacroDef {
            kind,
            name: name.to_owned(),
            opts: opts.map(str::to_owned),
            body,
            // Modifiers (`-e`, `-g`, `<l>`, `<o>`) are stage-2 work.
            eager: false,
            global: matches!(kind, MacroDefKind::Global),
            literal: false,
            one_shot: false,
            data: span,
        }),
    ))
}

fn parse_undefine<'a>(
    _state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%undefine").parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;
    let (after_name, name) = take_macro_name(after_kw)?;
    let (after_term, _) = line_terminator(after_name)?;
    let span = span_between(&start, &after_term);

    Ok((
        after_term,
        SpecItem::MacroDef(MacroDef {
            kind: MacroDefKind::Undefine,
            name: name.to_owned(),
            opts: None,
            body: Text::new(),
            eager: false,
            global: false,
            literal: false,
            one_shot: false,
            data: span,
        }),
    ))
}

fn parse_bcond<'a>(state: &ParserState, input: Input<'a>) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%bcond").parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;
    let (after_name, name) = take_macro_name(after_kw)?;
    let (after_name, _) = space0(after_name)?;
    let (after_default, default_raw) = match logical_line(after_name) {
        Ok(r) => r,
        Err(_) => (after_name, String::new()),
    };
    let trimmed = default_raw.trim();
    let default = if trimmed.is_empty() {
        None
    } else {
        Some(parse_body_as_text(state, trimmed))
    };
    let span = span_between(&start, &after_default);

    Ok((
        after_default,
        SpecItem::BuildCondition(BuildCondition {
            style: BuildCondStyle::Bcond,
            name: name.to_owned(),
            default,
            data: span,
        }),
    ))
}

fn parse_bcond_with<'a>(
    _state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%bcond_with").parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;
    let (after_name, name) = take_macro_name(after_kw)?;
    let (after_term, _) = line_terminator(after_name)?;
    let span = span_between(&start, &after_term);

    Ok((
        after_term,
        SpecItem::BuildCondition(BuildCondition {
            style: BuildCondStyle::BcondWith,
            name: name.to_owned(),
            default: None,
            data: span,
        }),
    ))
}

fn parse_bcond_without<'a>(
    _state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%bcond_without").parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;
    let (after_name, name) = take_macro_name(after_kw)?;
    let (after_term, _) = line_terminator(after_name)?;
    let span = span_between(&start, &after_term);

    Ok((
        after_term,
        SpecItem::BuildCondition(BuildCondition {
            style: BuildCondStyle::BcondWithout,
            name: name.to_owned(),
            default: None,
            data: span,
        }),
    ))
}

fn parse_include<'a>(state: &ParserState, input: Input<'a>) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%include").parse(input)?;
    let (after_kw, _) = require_space_after_keyword(after_kw)?;
    let (after_path, raw) = match logical_line(after_kw) {
        Ok(r) => r,
        Err(_) => (after_kw, String::new()),
    };
    let path = parse_body_as_text(state, raw.trim());
    let span = span_between(&start, &after_path);

    Ok((
        after_path,
        SpecItem::Include(IncludeDirective { path, data: span }),
    ))
}

fn parse_dnl<'a>(state: &ParserState, input: Input<'a>) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_kw, _) = tag("%dnl").parse(input)?;
    // `%dnl` swallows the rest of the line; no need for a space.
    let (after_text, text_raw) = match logical_line(after_kw) {
        Ok(r) => r,
        Err(_) => (after_kw, String::new()),
    };
    let body = parse_body_as_text(state, strip_leading_space(&text_raw));
    let span = span_between(&start, &after_text);

    Ok((
        after_text,
        SpecItem::Comment(Comment {
            style: CommentStyle::Dnl,
            text: body,
            data: span,
        }),
    ))
}

/// Parse a `#` comment line.
pub fn parse_hash_comment<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let start = input;
    let (after_hash, _) = space0(input)?;
    let after_hash = if after_hash.fragment().starts_with('#') {
        let (rest, _taken) = nom::Input::take_split(&after_hash, 1);
        rest
    } else {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    };
    let (after_text, text_raw) = match logical_line(after_hash) {
        Ok(r) => r,
        Err(_) => (after_hash, String::new()),
    };
    let body = parse_body_as_text(state, strip_leading_space(&text_raw));
    let span = span_between(&start, &after_text);

    Ok((
        after_text,
        SpecItem::Comment(Comment {
            style: CommentStyle::Hash,
            text: body,
            data: span,
        }),
    ))
}

/// Parse a bare top-level macro invocation as a [`SpecItem::Statement`].
///
/// This is the fallback for `%xxx` lines that are *not* a recognized macro
/// keyword and *not* a section header. The whole line is consumed.
pub fn parse_top_macro_call<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, SpecItem<Span>> {
    let (after_ws, _) = space0(input)?;
    let frag = *after_ws.fragment();
    if !frag.starts_with('%') {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }
    let (after_macro, m) = parse_macro_ref(state, after_ws)?;
    let (after_term, _) = line_terminator(after_macro)?;
    let _ = after_term;
    Ok((after_term, SpecItem::Statement(Box::new(m))))
}

fn require_space_after_keyword<'a>(input: Input<'a>) -> IResult<Input<'a>, ()> {
    let frag = *input.fragment();
    // Allow space or tab; reject newline/EOF because the caller still needs
    // an argument afterwards.
    match frag.chars().next() {
        Some(' ') | Some('\t') => {
            let (rest, _) = space0(input)?;
            Ok((rest, ()))
        }
        _ => Err(nom::Err::Error(error_position!(input, ErrorKind::Space))),
    }
}

fn take_macro_name<'a>(input: Input<'a>) -> IResult<Input<'a>, &'a str> {
    let frag = *input.fragment();
    let mut iter = frag.char_indices();
    let Some((_, first)) = iter.next() else {
        return Err(nom::Err::Error(error_position!(
            input,
            ErrorKind::AlphaNumeric
        )));
    };
    if !is_macro_name_start(first) {
        return Err(nom::Err::Error(error_position!(
            input,
            ErrorKind::AlphaNumeric
        )));
    }
    let mut end = first.len_utf8();
    for (i, c) in iter {
        if is_macro_name_char(c) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let (rest, _) = nom::Input::take_split(&input, end);
    Ok((rest, &frag[..end]))
}

fn take_optional_opts<'a>(input: Input<'a>) -> (Input<'a>, Option<&'a str>) {
    let frag = *input.fragment();
    if !frag.starts_with('(') {
        return (input, None);
    }
    // Balanced parens: simple linear scan because RPM opts are
    // single-level. If nesting appears in real specs, this needs to grow.
    let mut depth: usize = 0;
    let mut end_idx: Option<usize> = None;
    for (i, c) in frag.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end_idx = Some(i + c.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }
    match end_idx {
        Some(end) => {
            let (rest, _) = nom::Input::take_split(&input, end);
            (rest, Some(&frag[..end]))
        }
        None => (input, None),
    }
}

fn strip_leading_space(s: &str) -> &str {
    s.strip_prefix(' ')
        .or_else(|| s.strip_prefix('\t'))
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::TextSegment;

    fn run<F>(src: &str, f: F) -> (SpecItem<Span>, ParserState)
    where
        F: for<'a> Fn(&ParserState, Input<'a>) -> IResult<Input<'a>, SpecItem<Span>>,
    {
        let state = ParserState::new();
        let input = Input::new(src);
        let (_rest, item) = f(&state, input).unwrap();
        (item, state)
    }

    #[test]
    fn define_simple() {
        let (item, _) = run("%define foo bar\n", parse_top_macro_statement);
        match item {
            SpecItem::MacroDef(m) => {
                assert_eq!(m.name, "foo");
                assert_eq!(m.kind, MacroDefKind::Define);
                assert_eq!(m.body.literal_str(), Some("bar"));
            }
            other => panic!("expected MacroDef, got {other:?}"),
        }
    }

    #[test]
    fn global_simple() {
        let (item, _) = run("%global with_x 1\n", parse_top_macro_statement);
        match item {
            SpecItem::MacroDef(m) => {
                assert_eq!(m.name, "with_x");
                assert_eq!(m.kind, MacroDefKind::Global);
                assert!(m.global);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn define_with_opts() {
        let (item, _) = run(
            "%define greet(n:) Hello %{-n*}\n",
            parse_top_macro_statement,
        );
        match item {
            SpecItem::MacroDef(m) => {
                assert_eq!(m.name, "greet");
                assert_eq!(m.opts.as_deref(), Some("(n:)"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn define_multiline_body() {
        let (item, _) = run("%define foo a \\\nb \\\nc\n", parse_top_macro_statement);
        match item {
            SpecItem::MacroDef(m) => {
                // Body keeps `\n` between continued lines, no backslashes.
                assert_eq!(m.body.literal_str(), Some("a\nb\nc"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn undefine() {
        let (item, _) = run("%undefine some_macro\n", parse_top_macro_statement);
        match item {
            SpecItem::MacroDef(m) => {
                assert_eq!(m.name, "some_macro");
                assert_eq!(m.kind, MacroDefKind::Undefine);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bcond_with_default() {
        let (item, _) = run("%bcond openssl 1\n", parse_top_macro_statement);
        match item {
            SpecItem::BuildCondition(b) => {
                assert_eq!(b.style, BuildCondStyle::Bcond);
                assert_eq!(b.name, "openssl");
                assert_eq!(b.default.unwrap().literal_str(), Some("1"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bcond_with() {
        let (item, _) = run("%bcond_with openssl\n", parse_top_macro_statement);
        match item {
            SpecItem::BuildCondition(b) => {
                assert_eq!(b.style, BuildCondStyle::BcondWith);
                assert_eq!(b.name, "openssl");
                assert!(b.default.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bcond_without() {
        let (item, _) = run("%bcond_without gnutls\n", parse_top_macro_statement);
        match item {
            SpecItem::BuildCondition(b) => {
                assert_eq!(b.style, BuildCondStyle::BcondWithout);
                assert_eq!(b.name, "gnutls");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn include_path() {
        let (item, _) = run(
            "%include /etc/rpm/macros.fragment\n",
            parse_top_macro_statement,
        );
        match item {
            SpecItem::Include(inc) => {
                assert_eq!(inc.path.literal_str(), Some("/etc/rpm/macros.fragment"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn dnl_comment() {
        let (item, _) = run("%dnl this is invisible to rpm\n", parse_top_macro_statement);
        match item {
            SpecItem::Comment(c) => {
                assert_eq!(c.style, CommentStyle::Dnl);
                assert_eq!(c.text.literal_str(), Some("this is invisible to rpm"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn hash_comment() {
        let (item, _) = run("# workaround for bug #42\n", parse_hash_comment);
        match item {
            SpecItem::Comment(c) => {
                assert_eq!(c.style, CommentStyle::Hash);
                assert_eq!(c.text.literal_str(), Some("workaround for bug #42"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn hash_comment_with_macro() {
        let (item, _) = run("# uses %{name}\n", parse_hash_comment);
        match item {
            SpecItem::Comment(c) => {
                assert_eq!(c.style, CommentStyle::Hash);
                let segs = &c.text.segments;
                assert_eq!(segs.len(), 2);
                // Hash comments expand macros in RPM, so the AST records them.
                assert!(matches!(&segs[0], TextSegment::Literal(s) if s == "uses "));
                assert!(matches!(&segs[1], TextSegment::Macro(m) if m.name == "name"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn top_macro_call_dump() {
        let (item, _) = run("%dump\n", parse_top_macro_call);
        match item {
            SpecItem::Statement(m) => assert_eq!(m.name, "dump"),
            _ => panic!(),
        }
    }
}
