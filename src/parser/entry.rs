//! Top-level parser entry points and the recovery loop.

use crate::ast::{Comment, CommentStyle, Span, SpecFile, SpecItem, Text};
use crate::parse_result::{ParseResult, codes};

use super::cond::parse_conditional;
use super::input::{Input, span_at, span_between, span_for_line};
use super::macros::{parse_hash_comment, parse_top_macro_call, parse_top_macro_statement};
use super::preamble::parse_preamble_line;
use super::section::{parse_section, peek_section_header};
use super::state::ParserState;
use super::util::{blank_line, line_terminator, physical_line, space0, strip_bom};

/// Parse a spec string, discarding span information.
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input_len = input.len()))
)]
pub fn parse_str(input: &str) -> ParseResult<()> {
    let with_spans = parse_str_with_spans(input);
    let stripped = strip_spans(with_spans.spec);
    ParseResult {
        spec: stripped,
        diagnostics: with_spans.diagnostics,
    }
}

/// Parse a spec string and attach byte/line/column spans to every node.
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input_len = input.len()))
)]
pub fn parse_str_with_spans(input: &str) -> ParseResult<Span> {
    let state = ParserState::new();
    let cursor = strip_bom(Input::new(input));
    let mut items: Vec<SpecItem<Span>> = Vec::new();
    let mut cursor = cursor;
    let total_start = cursor;

    loop {
        if cursor.fragment().is_empty() {
            break;
        }

        // Standalone blank line.
        if let Ok((rest, _)) = blank_line(cursor) {
            if rest.location_offset() > cursor.location_offset() {
                items.push(SpecItem::Blank);
                cursor = rest;
                continue;
            }
        }

        // # comment.
        let after_ws_for_hash = match space0(cursor) {
            Ok((r, _)) => r,
            Err(_) => cursor,
        };
        if after_ws_for_hash.fragment().starts_with('#') {
            if let Ok((rest, item)) = parse_hash_comment(&state, cursor) {
                items.push(item);
                cursor = rest;
                continue;
            }
        }

        // Top-level macro statement.
        if let Ok((rest, item)) = parse_top_macro_statement(&state, cursor) {
            items.push(item);
            cursor = rest;
            continue;
        }

        // %if / %ifarch / %ifos block.
        if let Ok((rest, c)) = parse_conditional(&state, cursor, parse_top_level_item) {
            items.push(SpecItem::Conditional(c));
            cursor = rest;
            continue;
        }

        // Structural section header (Stage 2: %description, %package).
        match parse_section(&state, cursor) {
            Ok((rest, Some(section))) => {
                items.push(SpecItem::Section(Box::new(section)));
                cursor = rest;
                continue;
            }
            Ok((_rest, None)) => {
                // Header is recognized but its body is Stage 3 work —
                // swallow with a deferred-placeholder Comment.
                let name = peek_section_header(cursor).expect("header was recognized");
                let header_span = span_at(&cursor);
                let (after_section, _) = swallow_section_body(cursor);
                state.push_warning_code(
                    codes::W_DEFERRED_SECTION,
                    format!("section `{name}` parsing is not yet implemented; body skipped"),
                    Some(header_span),
                );
                items.push(SpecItem::Comment(Comment {
                    style: CommentStyle::Dnl,
                    text: Text::from(format!("[deferred] {name}")),
                    data: span_between(&cursor, &after_section),
                }));
                cursor = after_section;
                continue;
            }
            Err(_) => { /* not a section header; fall through */ }
        }

        // Preamble line.
        if let Ok((rest, mut new_items)) = parse_preamble_line(&state, cursor) {
            if !new_items.is_empty() {
                items.append(&mut new_items);
                cursor = rest;
                continue;
            }
        }

        // Bare `%foo` macro call as a statement.
        if let Ok((rest, item)) = parse_top_macro_call(&state, cursor) {
            items.push(item);
            cursor = rest;
            continue;
        }

        // Unrecognized line.
        let here = cursor;
        let (after_line, line_text) = match physical_line(cursor) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after_line.location_offset() == here.location_offset() {
            state.push_error_code(
                codes::E_NO_PROGRESS,
                "parser made no progress at this position",
                Some(span_at(&here)),
            );
            break;
        }
        state.push_warning_code(
            codes::W_LINE_NOT_RECOGNIZED,
            "line not recognized",
            // Span the *line content only*, not the trailing newline —
            // otherwise the carat in `codespan` output overlaps the
            // next physical line and confuses users (see P5 in the
            // P1-fix audit notes).
            Some(span_for_line(&here, &line_text)),
        );
        cursor = after_line;
    }

    let total_span = span_between(&total_start, &cursor);
    let spec = SpecFile {
        items,
        data: total_span,
    };
    let diagnostics = state.into_diagnostics();
    ParseResult { spec, diagnostics }
}

/// Body-item parser used by `parse_conditional` at the top level. Returns
/// `Vec<SpecItem>` because multi-dep preamble lines expand into multiple
/// items in one source-line step.
fn parse_top_level_item<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> nom::IResult<Input<'a>, Vec<SpecItem<Span>>> {
    if let Ok((rest, _)) = blank_line(input) {
        if rest.location_offset() > input.location_offset() {
            return Ok((rest, vec![SpecItem::Blank]));
        }
    }
    let after_ws = match space0(input) {
        Ok((r, _)) => r,
        Err(_) => input,
    };
    if after_ws.fragment().starts_with('#') {
        if let Ok((rest, item)) = parse_hash_comment(state, input) {
            return Ok((rest, vec![item]));
        }
    }
    if let Ok((rest, item)) = parse_top_macro_statement(state, input) {
        return Ok((rest, vec![item]));
    }
    if let Ok((rest, c)) = parse_conditional(state, input, parse_top_level_item) {
        return Ok((rest, vec![SpecItem::Conditional(c)]));
    }
    if let Ok((rest, items)) = parse_preamble_line(state, input) {
        if !items.is_empty() {
            return Ok((rest, items));
        }
    }
    let (rest, item) = parse_top_macro_call(state, input)?;
    Ok((rest, vec![item]))
}

/// Consume a section header line and the body that follows, up to (but
/// not including) the next section header. Used as a fallback for
/// section names whose structural parsing has not landed yet (Stage 3).
fn swallow_section_body(input: Input<'_>) -> (Input<'_>, ()) {
    // Skip the header line: prefer a clean `line_terminator` (trailing
    // whitespace + EOL), fall back to consuming the rest of the
    // physical line if the header is followed by junk before the EOL.
    let mut skip_header = nom::branch::alt((
        nom::combinator::map(line_terminator, |()| ()),
        nom::combinator::map(physical_line, |_| ()),
    ));
    let mut cursor = match nom::Parser::parse(&mut skip_header, input) {
        Ok((rest, ())) => rest,
        Err(_) => return (input, ()),
    };
    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let here = cursor;
        let (after, _) = match physical_line(here) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after.location_offset() == here.location_offset() {
            break;
        }
        cursor = after;
    }
    (cursor, ())
}

// ---------------------------------------------------------------------
// Span → () conversion
// ---------------------------------------------------------------------

fn strip_spans(file: SpecFile<Span>) -> SpecFile<()> {
    SpecFile {
        items: file.items.into_iter().map(strip_item).collect(),
        data: (),
    }
}

fn strip_item(item: SpecItem<Span>) -> SpecItem<()> {
    use crate::ast::{BuildCondition, Comment, IncludeDirective, MacroDef, PreambleItem};
    match item {
        SpecItem::Blank => SpecItem::Blank,
        SpecItem::Statement(m) => SpecItem::Statement(m),
        SpecItem::Include(IncludeDirective { path, .. }) => {
            SpecItem::Include(IncludeDirective { path, data: () })
        }
        SpecItem::Comment(Comment { style, text, .. }) => SpecItem::Comment(Comment {
            style,
            text,
            data: (),
        }),
        SpecItem::MacroDef(MacroDef {
            kind,
            name,
            opts,
            body,
            eager,
            global,
            literal,
            one_shot,
            ..
        }) => SpecItem::MacroDef(MacroDef {
            kind,
            name,
            opts,
            body,
            eager,
            global,
            literal,
            one_shot,
            data: (),
        }),
        SpecItem::BuildCondition(BuildCondition {
            style,
            name,
            default,
            ..
        }) => SpecItem::BuildCondition(BuildCondition {
            style,
            name,
            default,
            data: (),
        }),
        SpecItem::Conditional(c) => SpecItem::Conditional(strip_conditional(c)),
        SpecItem::Preamble(PreambleItem {
            tag,
            qualifiers,
            lang,
            value,
            ..
        }) => SpecItem::Preamble(PreambleItem {
            tag,
            qualifiers,
            lang,
            value,
            data: (),
        }),
        SpecItem::Section(s) => SpecItem::Section(Box::new(strip_section(*s))),
    }
}

fn strip_conditional(
    c: crate::ast::Conditional<Span, SpecItem<Span>>,
) -> crate::ast::Conditional<(), SpecItem<()>> {
    use crate::ast::{CondBranch, Conditional};
    Conditional {
        branches: c
            .branches
            .into_iter()
            .map(|b| CondBranch {
                kind: b.kind,
                expr: strip_cond_expr(b.expr),
                body: b.body.into_iter().map(strip_item).collect(),
                data: (),
            })
            .collect(),
        otherwise: c.otherwise.map(|v| v.into_iter().map(strip_item).collect()),
        data: (),
    }
}

fn strip_cond_expr(e: crate::ast::CondExpr<Span>) -> crate::ast::CondExpr<()> {
    use crate::ast::CondExpr;
    match e {
        CondExpr::Raw(t) => CondExpr::Raw(t),
        CondExpr::Parsed(ast) => CondExpr::Parsed(Box::new(strip_expr_ast(*ast))),
        CondExpr::ArchList(items) => CondExpr::ArchList(items),
    }
}

fn strip_expr_ast(ast: crate::ast::ExprAst<Span>) -> crate::ast::ExprAst<()> {
    use crate::ast::ExprAst;
    match ast {
        ExprAst::Integer { value, .. } => ExprAst::Integer { value, data: () },
        ExprAst::String { value, .. } => ExprAst::String { value, data: () },
        ExprAst::Macro { text, .. } => ExprAst::Macro { text, data: () },
        ExprAst::Identifier { name, .. } => ExprAst::Identifier { name, data: () },
        ExprAst::Paren { inner, .. } => ExprAst::Paren {
            inner: Box::new(strip_expr_ast(*inner)),
            data: (),
        },
        ExprAst::Not { inner, .. } => ExprAst::Not {
            inner: Box::new(strip_expr_ast(*inner)),
            data: (),
        },
        ExprAst::Binary { kind, lhs, rhs, .. } => ExprAst::Binary {
            kind,
            lhs: Box::new(strip_expr_ast(*lhs)),
            rhs: Box::new(strip_expr_ast(*rhs)),
            data: (),
        },
    }
}

fn strip_section(s: crate::ast::Section<Span>) -> crate::ast::Section<()> {
    use crate::ast::{ChangelogEntry, FileTrigger, Scriptlet, Section, Trigger};
    match s {
        Section::Description { subpkg, body, .. } => Section::Description {
            subpkg,
            body,
            data: (),
        },
        Section::Package {
            name_arg, content, ..
        } => Section::Package {
            name_arg,
            content: content.into_iter().map(strip_preamble_content).collect(),
            data: (),
        },
        Section::BuildScript { kind, body, .. } => Section::BuildScript {
            kind,
            body,
            data: (),
        },
        Section::Files {
            subpkg,
            file_lists,
            content,
            ..
        } => Section::Files {
            subpkg,
            file_lists,
            content: content.into_iter().map(strip_files_content).collect(),
            data: (),
        },
        Section::Scriptlet(Scriptlet {
            kind,
            subpkg,
            interp,
            expand_macros,
            quiet,
            from_file,
            body,
            ..
        }) => Section::Scriptlet(Scriptlet {
            kind,
            subpkg,
            interp,
            expand_macros,
            quiet,
            from_file,
            body,
            data: (),
        }),
        Section::Trigger(Trigger {
            kind,
            subpkg,
            interp,
            conditions,
            body,
            ..
        }) => Section::Trigger(Trigger {
            kind,
            subpkg,
            interp,
            conditions,
            body,
            data: (),
        }),
        Section::FileTrigger(FileTrigger {
            kind,
            subpkg,
            interp,
            priority,
            prefixes,
            body,
            ..
        }) => Section::FileTrigger(FileTrigger {
            kind,
            subpkg,
            interp,
            priority,
            prefixes,
            body,
            data: (),
        }),
        Section::Verify { subpkg, body, .. } => Section::Verify {
            subpkg,
            body,
            data: (),
        },
        Section::Changelog { entries, .. } => Section::Changelog {
            entries: entries
                .into_iter()
                .map(|e| ChangelogEntry {
                    date: e.date,
                    author: e.author,
                    email: e.email,
                    version: e.version,
                    body: e.body,
                    data: (),
                })
                .collect(),
            data: (),
        },
        Section::SourceList { entries, .. } => Section::SourceList { entries, data: () },
        Section::PatchList { entries, .. } => Section::PatchList { entries, data: () },
        Section::Sepolicy { subpkg, body, .. } => Section::Sepolicy {
            subpkg,
            body,
            data: (),
        },
    }
}

fn strip_files_content(fc: crate::ast::FilesContent<Span>) -> crate::ast::FilesContent<()> {
    use crate::ast::{Comment, FileEntry, FilesContent};
    match fc {
        FilesContent::Blank => FilesContent::Blank,
        FilesContent::Comment(Comment { style, text, .. }) => FilesContent::Comment(Comment {
            style,
            text,
            data: (),
        }),
        FilesContent::Entry(FileEntry {
            directives, path, ..
        }) => FilesContent::Entry(FileEntry {
            directives,
            path,
            data: (),
        }),
        FilesContent::Conditional(c) => FilesContent::Conditional(strip_files_conditional(c)),
    }
}

fn strip_files_conditional(
    c: crate::ast::Conditional<Span, crate::ast::FilesContent<Span>>,
) -> crate::ast::Conditional<(), crate::ast::FilesContent<()>> {
    use crate::ast::{CondBranch, Conditional};
    Conditional {
        branches: c
            .branches
            .into_iter()
            .map(|b| CondBranch {
                kind: b.kind,
                expr: strip_cond_expr(b.expr),
                body: b.body.into_iter().map(strip_files_content).collect(),
                data: (),
            })
            .collect(),
        otherwise: c
            .otherwise
            .map(|v| v.into_iter().map(strip_files_content).collect()),
        data: (),
    }
}

fn strip_preamble_content(
    pc: crate::ast::PreambleContent<Span>,
) -> crate::ast::PreambleContent<()> {
    use crate::ast::{Comment, PreambleContent, PreambleItem};
    match pc {
        PreambleContent::Blank => PreambleContent::Blank,
        PreambleContent::Comment(Comment { style, text, .. }) => {
            PreambleContent::Comment(Comment {
                style,
                text,
                data: (),
            })
        }
        PreambleContent::Item(PreambleItem {
            tag,
            qualifiers,
            lang,
            value,
            ..
        }) => PreambleContent::Item(PreambleItem {
            tag,
            qualifiers,
            lang,
            value,
            data: (),
        }),
        PreambleContent::Conditional(c) => {
            PreambleContent::Conditional(strip_preamble_conditional(c))
        }
    }
}

fn strip_preamble_conditional(
    c: crate::ast::Conditional<Span, crate::ast::PreambleContent<Span>>,
) -> crate::ast::Conditional<(), crate::ast::PreambleContent<()>> {
    use crate::ast::{CondBranch, Conditional};
    Conditional {
        branches: c
            .branches
            .into_iter()
            .map(|b| CondBranch {
                kind: b.kind,
                expr: strip_cond_expr(b.expr),
                body: b.body.into_iter().map(strip_preamble_content).collect(),
                data: (),
            })
            .collect(),
        otherwise: c
            .otherwise
            .map(|v| v.into_iter().map(strip_preamble_content).collect()),
        data: (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Section, Tag, TagValue};

    fn parse(src: &str) -> ParseResult<Span> {
        parse_str_with_spans(src)
    }

    #[test]
    fn empty_input() {
        let r = parse("");
        assert!(r.spec.items.is_empty());
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn bom_is_eaten() {
        let r = parse("\u{feff}%define foo bar\n");
        assert_eq!(r.spec.items.len(), 1);
        assert!(matches!(r.spec.items[0], SpecItem::MacroDef(_)));
    }

    #[test]
    fn blank_lines_kept() {
        let r = parse("\n\n");
        assert_eq!(r.spec.items.len(), 2);
        assert!(r.spec.items.iter().all(|i| matches!(i, SpecItem::Blank)));
    }

    #[test]
    fn mixed_definitions_and_comments() {
        let src = "\
# top comment with %{macro}\n\
%global with_x 1\n\
%define foo bar\n\
%bcond_with openssl\n\
%bcond_without gnutls\n\
";
        let r = parse(src);
        let kinds: Vec<&'static str> = r
            .spec
            .items
            .iter()
            .map(|i| match i {
                SpecItem::Comment(_) => "comment",
                SpecItem::MacroDef(_) => "macrodef",
                SpecItem::BuildCondition(_) => "bcond",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["comment", "macrodef", "macrodef", "bcond", "bcond"]);
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    }

    #[test]
    fn conditional_block_collected() {
        let src = "\
%if 0%{?fedora}\n\
%define a 1\n\
%else\n\
%define a 0\n\
%endif\n\
";
        let r = parse(src);
        assert_eq!(r.spec.items.len(), 1);
        assert!(matches!(r.spec.items[0], SpecItem::Conditional(_)));
    }

    #[test]
    fn preamble_lines_parsed_structurally() {
        let src = "Name: hello\nVersion: 1.0\n";
        let r = parse(src);
        let preamble_count = r
            .spec
            .items
            .iter()
            .filter(|i| matches!(i, SpecItem::Preamble(_)))
            .count();
        assert_eq!(preamble_count, 2);
        let no_unrecognized = r
            .diagnostics
            .iter()
            .all(|d| !d.message.contains("not recognized"));
        assert!(no_unrecognized, "{:?}", r.diagnostics);
    }

    #[test]
    fn description_section_parsed_structurally() {
        let src = "%description\nHello body.\n";
        let r = parse(src);
        assert_eq!(r.spec.items.len(), 1);
        match &r.spec.items[0] {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Description { body, .. } => {
                    assert_eq!(body.lines.len(), 1);
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn files_section_parsed_structurally() {
        // After Stage 3 there are no `not yet implemented` diagnostics
        // for a normal spec.
        let src = "%files\n/usr/bin/whatever\n";
        let r = parse(src);
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.message.contains("not yet implemented"))
        );
        assert!(matches!(
            r.spec.items.last().unwrap(),
            SpecItem::Section(s) if matches!(s.as_ref(), Section::Files { .. })
        ));
    }

    #[test]
    fn span_covers_macro_definition() {
        let src = "%define foo bar\n";
        let r = parse(src);
        let item = &r.spec.items[0];
        if let SpecItem::MacroDef(m) = item {
            assert_eq!(m.data.start_byte, 0);
            assert!(m.data.end_byte >= "%define foo bar".len());
            assert_eq!(m.data.start_line, 1);
        } else {
            panic!("expected MacroDef");
        }
    }

    #[test]
    fn full_minimal_spec() {
        let src = "\
Name: hello\n\
Version: 1.0\n\
Release: 1%{?dist}\n\
Summary: hi\n\
License: MIT\n\
\n\
%description\n\
Greets the world.\n\
\n\
%files\n\
/usr/bin/hello\n\
";
        let r = parse(src);
        let preambles = r
            .spec
            .items
            .iter()
            .filter(|i| matches!(i, SpecItem::Preamble(_)))
            .count();
        assert_eq!(preambles, 5);
        let descriptions = r
            .spec
            .items
            .iter()
            .filter(
                |i| matches!(i, SpecItem::Section(s) if matches!(**s, Section::Description{..})),
            )
            .count();
        assert_eq!(descriptions, 1);
        // After Stage 3 no sections are deferred anymore.
        let deferred = r
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("not yet implemented"))
            .count();
        assert_eq!(deferred, 0);
    }

    #[test]
    fn strip_spans_round_trip_through_parse_str() {
        let r = super::parse_str("Name: hello\n%description\nbody\n");
        let names: Vec<_> = r
            .spec
            .items
            .iter()
            .filter_map(|i| match i {
                SpecItem::Preamble(p) => match &p.value {
                    TagValue::Text(t) => Some((p.tag.clone(), t.literal_str().map(str::to_owned))),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            names
                .iter()
                .any(|(t, v)| matches!(t, Tag::Name) && v.as_deref() == Some("hello"))
        );
    }
}
