//! Parsers for section headers and the structural bodies that Stage 2
//! produces (`%description`, `%package`).
//!
//! Other section names (`%prep`, `%build`, `%files`, `%changelog`, …)
//! are detected here only for the purpose of stopping the parent body
//! parser — their structural bodies remain Stage 3 work and the
//! top-level loop in `entry.rs` falls back to the deferred-placeholder
//! path for them.

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{
    BuildScriptKind, PackageName, PreambleContent, Section, ShellBody, Span, SubpkgRef, Text,
    TextBody, TextSegment,
};
use crate::parse_result::codes;

use super::input::{Input, span_at, span_between, span_for_line};
use super::preamble::parse_preamble_content;
use super::state::ParserState;
use super::text::{parse_body_as_text, parse_text};
use super::util::{line_terminator, physical_line, space0, space1};

/// Section header names that introduce a top-level section. Order does
/// not matter except that longer-prefix names must be tried first when
/// they overlap (`%description` vs `%desc`) — none currently do.
pub(crate) const SECTION_HEADERS: &[&str] = &[
    "%description",
    "%package",
    "%prep",
    "%conf",
    "%build",
    "%install",
    "%check",
    "%clean",
    "%generate_buildrequires",
    "%files",
    "%changelog",
    "%sourcelist",
    "%patchlist",
    "%verify",
    "%sepolicy",
    "%pre",
    "%post",
    "%preun",
    "%postun",
    "%pretrans",
    "%posttrans",
    "%preuntrans",
    "%postuntrans",
    "%triggerprein",
    "%triggerin",
    "%triggerun",
    "%triggerpostun",
    "%filetriggerin",
    "%filetriggerun",
    "%filetriggerpostun",
    "%transfiletriggerin",
    "%transfiletriggerun",
    "%transfiletriggerpostun",
];

/// Returns the canonical name (e.g. `"%description"`) when the cursor
/// (after any leading whitespace) sits on a recognized section header,
/// else `None`. Section names must be followed by whitespace, EOL, EOF,
/// or `-` (option).
pub fn peek_section_header(input: Input<'_>) -> Option<&'static str> {
    let after_ws = match space0(input) {
        Ok((r, _)) => r,
        Err(_) => return None,
    };
    let frag = *after_ws.fragment();
    for header in SECTION_HEADERS {
        if let Some(rest) = frag.strip_prefix(header) {
            match rest.chars().next() {
                None | Some(' ' | '\t' | '\n' | '\r' | '-') => return Some(*header),
                _ => {}
            }
        }
    }
    None
}

/// Parse a structural section if the cursor sits on a section header that
/// Stage 2 knows how to handle. Returns `Ok((rest, None))` when the
/// header is recognized but its body is *not* yet implemented (the
/// caller should fall back to the deferred-placeholder path).
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "trace", skip(state, input))
)]
pub fn parse_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Option<Section<Span>>> {
    let header = match peek_section_header(input) {
        Some(h) => h,
        None => {
            return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
        }
    };
    match header {
        "%description" => {
            let (rest, sec) = parse_description_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%package" => {
            let (rest, sec) = parse_package_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%prep" => parse_build_script(state, input, "%prep", BuildScriptKind::Prep).map(some),
        "%conf" => parse_build_script(state, input, "%conf", BuildScriptKind::Conf).map(some),
        "%build" => parse_build_script(state, input, "%build", BuildScriptKind::Build).map(some),
        "%install" => {
            parse_build_script(state, input, "%install", BuildScriptKind::Install).map(some)
        }
        "%check" => parse_build_script(state, input, "%check", BuildScriptKind::Check).map(some),
        "%clean" => parse_build_script(state, input, "%clean", BuildScriptKind::Clean).map(some),
        "%generate_buildrequires" => parse_build_script(
            state,
            input,
            "%generate_buildrequires",
            BuildScriptKind::GenerateBuildRequires,
        )
        .map(some),
        "%verify" => parse_verify_section(state, input).map(some),
        "%sepolicy" => parse_sepolicy_section(state, input).map(some),
        "%sourcelist" => {
            parse_list_section(state, input, "%sourcelist", ListKind::Source).map(some)
        }
        "%patchlist" => parse_list_section(state, input, "%patchlist", ListKind::Patch).map(some),
        "%files" => {
            let (rest, sec) = super::files::parse_files_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%changelog" => {
            let (rest, sec) = super::changelog::parse_changelog_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%pre" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%pre",
            crate::ast::ScriptletKind::Pre,
        )
        .map(some),
        "%post" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%post",
            crate::ast::ScriptletKind::Post,
        )
        .map(some),
        "%preun" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%preun",
            crate::ast::ScriptletKind::Preun,
        )
        .map(some),
        "%postun" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%postun",
            crate::ast::ScriptletKind::Postun,
        )
        .map(some),
        "%pretrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%pretrans",
            crate::ast::ScriptletKind::Pretrans,
        )
        .map(some),
        "%posttrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%posttrans",
            crate::ast::ScriptletKind::Posttrans,
        )
        .map(some),
        "%preuntrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%preuntrans",
            crate::ast::ScriptletKind::Preuntrans,
        )
        .map(some),
        "%postuntrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%postuntrans",
            crate::ast::ScriptletKind::Postuntrans,
        )
        .map(some),
        "%triggerprein" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerprein",
            crate::ast::TriggerKind::Prein,
        )
        .map(some),
        "%triggerin" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerin",
            crate::ast::TriggerKind::In,
        )
        .map(some),
        "%triggerun" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerun",
            crate::ast::TriggerKind::Un,
        )
        .map(some),
        "%triggerpostun" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerpostun",
            crate::ast::TriggerKind::Postun,
        )
        .map(some),
        "%filetriggerin" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerin",
            crate::ast::FileTriggerKind::In,
        )
        .map(some),
        "%filetriggerun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerun",
            crate::ast::FileTriggerKind::Un,
        )
        .map(some),
        "%filetriggerpostun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerpostun",
            crate::ast::FileTriggerKind::Postun,
        )
        .map(some),
        "%transfiletriggerin" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerin",
            crate::ast::FileTriggerKind::TransIn,
        )
        .map(some),
        "%transfiletriggerun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerun",
            crate::ast::FileTriggerKind::TransUn,
        )
        .map(some),
        "%transfiletriggerpostun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerpostun",
            crate::ast::FileTriggerKind::TransPostun,
        )
        .map(some),
        _ => Ok((input, None)),
    }
}

fn some<I, T>((rest, value): (I, T)) -> (I, Option<T>) {
    (rest, Some(value))
}

// ---------------------------------------------------------------------
// Shell-body helper shared by build-scripts/scriptlets/triggers/etc.
// ---------------------------------------------------------------------

/// Consume body lines (one `Text` per physical line, with macros parsed)
/// until the next recognized section header or EOF. Used by build-script
/// and scriptlet/trigger sections.
pub(crate) fn collect_shell_body_until_section_header<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, ShellBody) {
    let mut cursor = input;
    let mut lines: Vec<Text> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let here = cursor;
        let (after, line_input) = match physical_line(here) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after.location_offset() == here.location_offset() {
            break;
        }
        let line = parse_body_as_text(state, line_input.fragment());
        lines.push(line);
        cursor = after;
    }

    // Trim trailing empty lines.
    while matches!(lines.last(), Some(t) if is_empty_text(t)) {
        lines.pop();
    }

    (cursor, ShellBody { lines })
}

// ---------------------------------------------------------------------
// Build-script handlers (%prep / %conf / %build / %install / %check /
// %clean / %generate_buildrequires)
// ---------------------------------------------------------------------

fn parse_build_script<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: BuildScriptKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    // Build-scripts have no header args in practice; consume the rest of
    // the header line (which may include `# trailing comment`).
    let (after_header, _) = line_terminator(after_kw)?;

    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::BuildScript {
            kind,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %verify
// ---------------------------------------------------------------------

fn parse_verify_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%verify".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::Verify {
            subpkg,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %sepolicy
// ---------------------------------------------------------------------

fn parse_sepolicy_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%sepolicy".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::Sepolicy {
            subpkg,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %sourcelist / %patchlist
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Source,
    Patch,
}

fn parse_list_section<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: ListKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    let (after_header, _) = line_terminator(after_kw)?;

    let mut cursor = after_header;
    let mut entries: Vec<Text> = Vec::new();
    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let here = cursor;
        let (after, line_input) = match physical_line(here) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after.location_offset() == here.location_offset() {
            break;
        }
        let frag = line_input.fragment().trim();
        if !frag.is_empty() && !frag.starts_with('#') {
            entries.push(parse_body_as_text(state, frag));
        }
        cursor = after;
    }

    let span = span_between(&start, &cursor);
    Ok((
        cursor,
        match kind {
            ListKind::Source => Section::SourceList {
                entries,
                data: span,
            },
            ListKind::Patch => Section::PatchList {
                entries,
                data: span,
            },
        },
    ))
}

// ---------------------------------------------------------------------
// %description
// ---------------------------------------------------------------------

fn parse_description_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%description".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;

    let (after_body, body) = collect_text_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Description {
            subpkg,
            body,
            data: span,
        },
    ))
}

fn collect_text_body_until_section_header<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, TextBody) {
    let mut cursor = input;
    let mut lines: Vec<Text> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let line_start = cursor;
        let (after_line_content, line_input) = match physical_line(cursor) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after_line_content.location_offset() == line_start.location_offset() {
            break;
        }
        // Parse the textual portion of this physical line as a Text.
        let line_text = parse_body_as_text(state, line_input.fragment());
        lines.push(line_text);
        cursor = after_line_content;
    }

    // Trim trailing empty lines (cosmetic: a body that immediately
    // precedes the next section header would otherwise carry a stray
    // empty line just to satisfy the source separator).
    while matches!(lines.last(), Some(t) if is_empty_text(t)) {
        lines.pop();
    }

    (cursor, TextBody { lines })
}

fn is_empty_text(t: &Text) -> bool {
    t.segments
        .iter()
        .all(|s| matches!(s, TextSegment::Literal(s) if s.trim().is_empty()))
}

// ---------------------------------------------------------------------
// %package
// ---------------------------------------------------------------------

fn parse_package_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%package".len());

    // %package requires a name argument.
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let name_arg = match subpkg {
        Some(SubpkgRef::Absolute(t)) => PackageName::Absolute(t),
        Some(SubpkgRef::Relative(t)) => PackageName::Relative(t),
        None => {
            state.push_error_code(
                codes::E_PACKAGE_NEEDS_NAME,
                "%package requires a subpackage name argument",
                Some(span_at(&after_args)),
            );
            // Recover with an empty name so the rest of the file still
            // parses; consumers see the diagnostic.
            PackageName::Relative(Text::new())
        }
    };
    let (after_header, _) = line_terminator(after_args)?;

    let (after_body, content) = collect_package_body(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Package {
            name_arg,
            content,
            data: span,
        },
    ))
}

fn collect_package_body<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Vec<PreambleContent<Span>>) {
    let mut cursor = input;
    let mut content: Vec<PreambleContent<Span>> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        match parse_preamble_content(state, cursor) {
            Ok((rest, items)) => {
                if rest.location_offset() == cursor.location_offset() {
                    // No progress — bail to avoid infinite loop.
                    break;
                }
                content.extend(items);
                cursor = rest;
            }
            Err(_) => {
                // Unrecognized line inside %package body — consume one
                // physical line with a warning so the body parser stays
                // productive.
                let here = cursor;
                let (after, line_text) = match physical_line(here) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                if after.location_offset() == here.location_offset() {
                    break;
                }
                state.push_warning_code(
                    codes::W_LINE_NOT_RECOGNIZED_IN_PACKAGE,
                    "line not recognized inside %package body",
                    Some(span_for_line(&here, &line_text)),
                );
                cursor = after;
            }
        }
    }

    (cursor, content)
}

// ---------------------------------------------------------------------
// `-n NAME` and bare-NAME header argument parsing
// ---------------------------------------------------------------------

fn parse_header_args<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Option<SubpkgRef>) {
    // Consume optional inline whitespace.
    let (cursor, _) = match space0(input) {
        Ok(r) => r,
        Err(_) => (input, input),
    };
    let frag = *cursor.fragment();

    if frag.starts_with("-n") {
        let after_flag = match advance_str(cursor, "-n".len()) {
            Some(a) => a,
            None => return (cursor, None),
        };
        let (after_ws, _) = match space1(after_flag) {
            Ok(r) => r,
            Err(_) => return (cursor, None),
        };
        match take_name_with_macros(state, after_ws) {
            Some((after_name, name)) => (after_name, Some(SubpkgRef::Absolute(name))),
            None => (cursor, None),
        }
    } else if frag.is_empty() || frag.starts_with('\n') || frag.starts_with('\r') {
        // No args at all — e.g. `%description` for the main package.
        (cursor, None)
    } else {
        match take_name_with_macros(state, cursor) {
            Some((after_name, name)) => (after_name, Some(SubpkgRef::Relative(name))),
            None => (cursor, None),
        }
    }
}

/// Parse a section name argument that may contain macro references like
/// `%{shortname}-sub1`. Stops at whitespace, EOL, or EOF. Returns `None`
/// when the cursor sits on a terminator (no name to consume).
pub(crate) fn take_name_with_macros<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> Option<(Input<'a>, Text)> {
    let frag = *input.fragment();
    let first = frag.chars().next()?;
    if matches!(first, ' ' | '\t' | '\n' | '\r') {
        return None;
    }
    let is_terminator = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    match parse_text(state, input, &is_terminator) {
        Ok((rest, text)) => {
            if text.segments.is_empty() {
                None
            } else {
                Some((rest, text))
            }
        }
        Err(_) => None,
    }
}

fn advance_str<'a>(input: Input<'a>, n: usize) -> Option<Input<'a>> {
    if input.fragment().len() < n {
        return None;
    }
    let (rest, _) = nom::Input::take_split(&input, n);
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{PreambleContent, Tag};

    fn parse(src: &str) -> Section<Span> {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_section(&state, inp).unwrap();
        sec.expect("section recognized")
    }

    #[test]
    fn description_main() {
        let s = parse("%description\nLine one.\nLine two.\n");
        match s {
            Section::Description { subpkg, body, .. } => {
                assert!(subpkg.is_none());
                assert_eq!(body.lines.len(), 2);
                assert_eq!(body.lines[0].literal_str(), Some("Line one."));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_relative() {
        let s = parse("%description foo\nText body.\n");
        match s {
            Section::Description { subpkg, body, .. } => {
                match subpkg.unwrap() {
                    SubpkgRef::Relative(t) => assert_eq!(t.literal_str(), Some("foo")),
                    _ => panic!(),
                }
                assert_eq!(body.lines.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_absolute() {
        let s = parse("%description -n libfoo\nhi\n");
        match s {
            Section::Description { subpkg, .. } => match subpkg.unwrap() {
                SubpkgRef::Absolute(t) => assert_eq!(t.literal_str(), Some("libfoo")),
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_with_macro_suffix() {
        // Regression: real-world specs like texlive-base.spec use
        // `%description -n %{shortname}-sub1`. The header argument must
        // accept macro segments, not only literal identifiers.
        let s = parse(
            "%description -n %{shortname}-sub1\nbody one\nbody two\n",
        );
        match s {
            Section::Description { subpkg, body, .. } => {
                match subpkg.expect("subpkg parsed") {
                    SubpkgRef::Absolute(t) => {
                        assert_eq!(t.segments.len(), 2);
                        assert!(matches!(&t.segments[0], TextSegment::Macro(_)));
                        assert!(
                            matches!(&t.segments[1], TextSegment::Literal(s) if s == "-sub1")
                        );
                    }
                    _ => panic!(),
                }
                assert_eq!(body.lines.len(), 2);
                assert_eq!(body.lines[0].literal_str(), Some("body one"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_subpkg_with_macro_suffix() {
        let s = parse("%package -n %{shortname}-sub1\nSummary: x\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                match name_arg {
                    PackageName::Absolute(t) => {
                        assert_eq!(t.segments.len(), 2);
                        assert!(matches!(&t.segments[0], TextSegment::Macro(_)));
                    }
                    _ => panic!(),
                }
                assert_eq!(content.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_stops_at_next_section() {
        let s = parse("%description\nbody1\nbody2\n%files\n/path\n");
        match s {
            Section::Description { body, .. } => {
                assert_eq!(body.lines.len(), 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_with_preamble() {
        let s = parse("%package foo\nSummary: Foo subpkg\nRequires: bar\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                match name_arg {
                    PackageName::Relative(t) => assert_eq!(t.literal_str(), Some("foo")),
                    _ => panic!(),
                }
                assert_eq!(content.len(), 2);
                match &content[0] {
                    PreambleContent::Item(p) => assert!(matches!(p.tag, Tag::Summary)),
                    other => panic!("{other:?}"),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_absolute_name() {
        let s = parse("%package -n libfoo\nLicense: MIT\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                assert!(matches!(name_arg, PackageName::Absolute(_)));
                assert_eq!(content.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_body_with_comment_and_blank() {
        let s = parse("%package foo\n# a comment\n\nSummary: X\n");
        match s {
            Section::Package { content, .. } => {
                assert_eq!(content.len(), 3);
                assert!(matches!(content[0], PreambleContent::Comment(_)));
                assert!(matches!(content[1], PreambleContent::Blank));
                assert!(matches!(content[2], PreambleContent::Item(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn peek_section_returns_some_for_known() {
        let inp = Input::new("%description hi\n");
        assert_eq!(peek_section_header(inp), Some("%description"));
    }

    #[test]
    fn peek_section_returns_none_for_other() {
        let inp = Input::new("Name: hello\n");
        assert!(peek_section_header(inp).is_none());
    }
}
