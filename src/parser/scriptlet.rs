//! Parsers for scriptlets, triggers, and file-triggers.
//!
//! Header grammar (options may appear in any order):
//!
//! ```text
//! scriptlet := '%<kind>' [SUBPKG] header-opts*
//! trigger   := '%<kind>' [SUBPKG] header-opts* '--' COND (',' COND)*
//! filetrig  := '%<kind>' [SUBPKG] header-opts* [-P NN] '--' PREFIX (',' PREFIX)*
//!
//! header-opts := '-n' NAME
//!              | '-p' INTERP
//!              | '-e'
//!              | '-q'
//!              | '-f' FILE
//!              | '-P' PRIORITY              (file-triggers only)
//! ```
//!
//! Bare `NAME` as the first token before any flag is taken as the relative
//! subpkg name (`SubpkgRef::Relative`); `-n NAME` produces the absolute
//! form. `-p <lua>` (literal angle brackets) yields [`Interpreter::Lua`].

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{
    DepExpr, FileTrigger, FileTriggerKind, Interpreter, Scriptlet, ScriptletKind, Section, Span,
    SubpkgRef, Text, Trigger, TriggerKind,
};
use crate::parse_result::codes;

use super::deps::parse_dep_expr;
use super::input::{Input, span_at, span_between};
use super::preamble::split_dep_list;
use super::section::{collect_shell_body_until_section_header, take_name_with_macros};
use super::state::ParserState;
use super::text::parse_body_as_text;
use super::util::{line_terminator, space0};

#[derive(Debug, Default)]
struct HeaderOpts {
    subpkg: Option<SubpkgRef>,
    interp: Option<Interpreter>,
    expand_macros: bool,
    quiet: bool,
    from_file: Option<Text>,
    priority: Option<u32>,
    /// Raw conditions/prefixes following `--`. Caller decodes them into
    /// either dep conditions (triggers) or prefix paths (file-triggers).
    after_dashes: Option<String>,
}

/// Parse a scriptlet section header and shell body into
/// [`Section::Scriptlet`]. `keyword` is the literal section name
/// (e.g. `"%post"`).
pub fn parse_scriptlet_section<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: ScriptletKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    let (after_header, opts) = parse_header(state, after_kw, /*for_trigger=*/ false)?;
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);

    if opts.after_dashes.is_some() {
        state.push_warning_code(
            codes::W_SCRIPTLET_DASHES_INVALID,
            "`--` separator is not valid for a scriptlet header; ignored",
            Some(span_at(&after_kw)),
        );
    }

    Ok((
        after_body,
        Section::Scriptlet(Scriptlet {
            kind,
            subpkg: opts.subpkg,
            interp: opts.interp,
            expand_macros: opts.expand_macros,
            quiet: opts.quiet,
            from_file: opts.from_file,
            body,
            data: span,
        }),
    ))
}

/// Parse a `%trigger*` section.
pub fn parse_trigger_section<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: TriggerKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    let (after_header, opts) = parse_header(state, after_kw, /*for_trigger=*/ true)?;
    let conditions = match opts.after_dashes.as_deref() {
        Some(raw) => parse_conditions(state, raw),
        None => {
            state.push_warning_code(
                codes::W_TRIGGER_MISSING_DASHES,
                format!("trigger `{keyword}` is missing `--` conditions"),
                Some(span_at(&after_kw)),
            );
            Vec::new()
        }
    };
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Trigger(Trigger {
            kind,
            subpkg: opts.subpkg,
            interp: opts.interp,
            conditions,
            body,
            data: span,
        }),
    ))
}

/// Parse a `%filetrigger*` or `%transfiletrigger*` section.
pub fn parse_file_trigger_section<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: FileTriggerKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    let (after_header, opts) = parse_header(state, after_kw, /*for_trigger=*/ true)?;
    let prefixes = match opts.after_dashes.as_deref() {
        Some(raw) => parse_prefixes(state, raw),
        None => {
            state.push_warning_code(
                codes::W_FILE_TRIGGER_MISSING_DASHES,
                format!("file-trigger `{keyword}` is missing `--` prefixes"),
                Some(span_at(&after_kw)),
            );
            Vec::new()
        }
    };
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::FileTrigger(FileTrigger {
            kind,
            subpkg: opts.subpkg,
            interp: opts.interp,
            priority: opts.priority,
            prefixes,
            body,
            data: span,
        }),
    ))
}

// ---------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------

fn parse_header<'a>(
    state: &ParserState,
    input: Input<'a>,
    for_trigger: bool,
) -> IResult<Input<'a>, HeaderOpts> {
    let mut cursor = input;
    let mut opts = HeaderOpts::default();

    loop {
        let (after_ws, _) = space0(cursor)?;
        cursor = after_ws;
        let frag = *cursor.fragment();
        if frag.is_empty() || frag.starts_with('\n') || frag.starts_with('\r') {
            break;
        }

        if frag.starts_with("--") {
            // `--` separator + the remainder of the line.
            cursor = advance(cursor, 2)
                .ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            // Capture the rest of the physical line.
            let rest_frag = *cursor.fragment();
            let nl_idx = rest_frag.find(['\n', '\r']).unwrap_or(rest_frag.len());
            opts.after_dashes = Some(rest_frag[..nl_idx].trim_end().to_owned());
            cursor = advance(cursor, nl_idx).unwrap_or(cursor);
            break;
        }

        if let Some(after_n) = strip_flag(frag, "-n") {
            cursor = advance(cursor, frag.len() - after_n.len())
                .ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_name_with_macros(state, cursor) {
                Some((after_name, name)) => {
                    opts.subpkg = Some(SubpkgRef::Absolute(name));
                    cursor = after_name;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_NAME_AFTER_N,
                        "scriptlet header: expected NAME after -n",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if let Some(_after_p) = strip_flag(frag, "-p") {
            cursor = advance(cursor, 2)
                .ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_interp_token(cursor) {
                Some((after_interp, raw)) => {
                    opts.interp = Some(parse_interp(state, raw));
                    cursor = after_interp;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_INTERP,
                        "scriptlet header: expected INTERP after -p",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if frag.starts_with("-e") && is_flag_boundary(frag, 2) {
            cursor = advance(cursor, 2).expect("checked length");
            opts.expand_macros = true;
        } else if frag.starts_with("-q") && is_flag_boundary(frag, 2) {
            cursor = advance(cursor, 2).expect("checked length");
            opts.quiet = true;
        } else if let Some(_after_f) = strip_flag(frag, "-f") {
            cursor = advance(cursor, 2).expect("checked length");
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_path_token(cursor) {
                Some((after_path, raw)) => {
                    opts.from_file = Some(parse_body_as_text(state, raw));
                    cursor = after_path;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_FILE_AFTER_F,
                        "scriptlet header: expected FILE after -f",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if let Some(_after_pp) = strip_flag(frag, "-P") {
            cursor = advance(cursor, 2).expect("checked length");
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_unsigned_token(cursor) {
                Some((after_num, n)) => {
                    opts.priority = Some(n);
                    cursor = after_num;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_PRIORITY,
                        "scriptlet header: expected priority after -P",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if opts.subpkg.is_none() && !frag.starts_with('-') {
            // Bare subpackage name (relative form) — only allowed as the
            // first non-flag token.
            match take_name_with_macros(state, cursor) {
                Some((after_name, name)) => {
                    opts.subpkg = Some(SubpkgRef::Relative(name));
                    cursor = after_name;
                }
                None => break,
            }
        } else if frag.starts_with('#') {
            // Trailing comment in the header line — stop here, line_terminator
            // will eat it.
            break;
        } else {
            // Unknown flag or stray token — warn and break to avoid loops.
            state.push_warning_code(
                codes::W_UNKNOWN_SCRIPTLET_TOKEN,
                format!("unknown scriptlet header token at `{}`", first_word(frag)),
                Some(span_at(&cursor)),
            );
            break;
        }
    }

    let _ = for_trigger; // currently only used to gate diagnostics on `--`
    let (after_term, _) = line_terminator(cursor)?;
    Ok((after_term, opts))
}

fn parse_conditions(state: &ParserState, raw: &str) -> Vec<DepExpr> {
    split_dep_list(raw)
        .iter()
        .filter_map(|slice| parse_dep_expr(state, slice).ok())
        .collect()
}

fn parse_prefixes(state: &ParserState, raw: &str) -> Vec<Text> {
    // Prefixes are comma-separated paths (rpm-locate spec).
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| parse_body_as_text(state, s))
        .collect()
}

fn parse_interp(state: &ParserState, raw: &str) -> Interpreter {
    if raw == "<lua>" {
        Interpreter::Lua
    } else {
        Interpreter::Path(parse_body_as_text(state, raw))
    }
}

// ---------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------

fn strip_flag<'a>(frag: &'a str, flag: &str) -> Option<&'a str> {
    if !frag.starts_with(flag) {
        return None;
    }
    let after = &frag[flag.len()..];
    if !is_flag_boundary(frag, flag.len()) {
        return None;
    }
    Some(after)
}

fn is_flag_boundary(frag: &str, idx: usize) -> bool {
    matches!(
        frag.as_bytes().get(idx).copied(),
        Some(b' ' | b'\t' | b'\n' | b'\r') | None
    )
}

fn take_path_token<'a>(input: Input<'a>) -> Option<(Input<'a>, &'a str)> {
    let frag = *input.fragment();
    let mut iter = frag.char_indices();
    let (_, first) = iter.next()?;
    if matches!(first, ' ' | '\t' | '\n' | '\r') {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in iter {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            break;
        }
        end = i + c.len_utf8();
    }
    let (rest, _) = nom::Input::take_split(&input, end);
    Some((rest, &frag[..end]))
}

/// Read `-p` argument: either a `<lua>` literal or a path-like token.
fn take_interp_token<'a>(input: Input<'a>) -> Option<(Input<'a>, &'a str)> {
    let frag = *input.fragment();
    if let Some(rest_after_lua) = frag.strip_prefix("<lua>") {
        let _ = rest_after_lua;
        let (rest, _) = nom::Input::take_split(&input, "<lua>".len());
        return Some((rest, "<lua>"));
    }
    take_path_token(input)
}

fn take_unsigned_token<'a>(input: Input<'a>) -> Option<(Input<'a>, u32)> {
    let (rest, raw) = take_path_token(input)?;
    raw.parse::<u32>().ok().map(|n| (rest, n))
}

fn advance<'a>(input: Input<'a>, n: usize) -> Option<Input<'a>> {
    if input.fragment().len() < n {
        return None;
    }
    let (rest, _) = nom::Input::take_split(&input, n);
    Some(rest)
}

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_scriptlet(
        src: &str,
        keyword: &str,
        kind: ScriptletKind,
    ) -> (Section<Span>, ParserState) {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_scriptlet_section(&state, inp, keyword, kind).unwrap();
        (sec, state)
    }

    fn run_trigger(src: &str, keyword: &str, kind: TriggerKind) -> (Section<Span>, ParserState) {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_trigger_section(&state, inp, keyword, kind).unwrap();
        (sec, state)
    }

    fn run_file_trigger(
        src: &str,
        keyword: &str,
        kind: FileTriggerKind,
    ) -> (Section<Span>, ParserState) {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_file_trigger_section(&state, inp, keyword, kind).unwrap();
        (sec, state)
    }

    fn scriptlet(sec: &Section<Span>) -> &Scriptlet<Span> {
        match sec {
            Section::Scriptlet(s) => s,
            _ => panic!("expected Scriptlet, got {sec:?}"),
        }
    }

    #[test]
    fn post_bare() {
        let (sec, _) = run_scriptlet("%post\necho hi\n", "%post", ScriptletKind::Post);
        let s = scriptlet(&sec);
        assert!(s.subpkg.is_none());
        assert!(s.interp.is_none());
        assert_eq!(s.body.lines.len(), 1);
    }

    #[test]
    fn post_with_interpreter_path() {
        let (sec, _) = run_scriptlet("%post -p /sbin/ldconfig\n", "%post", ScriptletKind::Post);
        let s = scriptlet(&sec);
        match s.interp.as_ref().unwrap() {
            Interpreter::Path(t) => assert_eq!(t.literal_str(), Some("/sbin/ldconfig")),
            _ => panic!(),
        }
        assert!(s.body.lines.is_empty());
    }

    #[test]
    fn post_with_lua_interpreter() {
        let (sec, _) = run_scriptlet(
            "%post -p <lua>\nprint('hi')\n",
            "%post",
            ScriptletKind::Post,
        );
        let s = scriptlet(&sec);
        assert!(matches!(s.interp, Some(Interpreter::Lua)));
        assert_eq!(s.body.lines.len(), 1);
    }

    #[test]
    fn post_bare_subpkg() {
        let (sec, _) = run_scriptlet("%post libfoo\necho hi\n", "%post", ScriptletKind::Post);
        let s = scriptlet(&sec);
        match s.subpkg.as_ref().unwrap() {
            SubpkgRef::Relative(t) => assert_eq!(t.literal_str(), Some("libfoo")),
            _ => panic!(),
        }
    }

    #[test]
    fn post_absolute_subpkg() {
        let (sec, _) = run_scriptlet("%post -n libfoo\necho hi\n", "%post", ScriptletKind::Post);
        let s = scriptlet(&sec);
        match s.subpkg.as_ref().unwrap() {
            SubpkgRef::Absolute(t) => assert_eq!(t.literal_str(), Some("libfoo")),
            _ => panic!(),
        }
    }

    #[test]
    fn post_with_flags_eqf() {
        let (sec, _) = run_scriptlet(
            "%post -e -q -f /tmp/body.sh\n",
            "%post",
            ScriptletKind::Post,
        );
        let s = scriptlet(&sec);
        assert!(s.expand_macros);
        assert!(s.quiet);
        assert_eq!(
            s.from_file.as_ref().unwrap().literal_str(),
            Some("/tmp/body.sh")
        );
    }

    #[test]
    fn trigger_with_conditions() {
        let (sec, _) = run_trigger(
            "%triggerin -- foo, bar >= 1.0\necho t\n",
            "%triggerin",
            TriggerKind::In,
        );
        let t = match &sec {
            Section::Trigger(t) => t,
            _ => panic!(),
        };
        assert_eq!(t.conditions.len(), 2);
        assert_eq!(t.body.lines.len(), 1);
    }

    #[test]
    fn trigger_missing_dashes_warns() {
        let state = ParserState::new();
        let inp = Input::new("%triggerin\necho t\n");
        let (_rest, _sec) =
            parse_trigger_section(&state, inp, "%triggerin", TriggerKind::In).unwrap();
        assert!(
            state
                .snapshot_diagnostics()
                .iter()
                .any(|d| d.message.contains("missing `--`"))
        );
    }

    #[test]
    fn file_trigger_with_prefixes_and_priority() {
        let (sec, _) = run_file_trigger(
            "%filetriggerin -P 200 -- /usr/lib, /usr/local/lib\ndo-it\n",
            "%filetriggerin",
            FileTriggerKind::In,
        );
        let ft = match &sec {
            Section::FileTrigger(ft) => ft,
            _ => panic!(),
        };
        assert_eq!(ft.priority, Some(200));
        assert_eq!(ft.prefixes.len(), 2);
    }

    #[test]
    fn body_stops_at_next_section() {
        let (sec, _) = run_scriptlet(
            "%post\nfirst\nsecond\n%files\n/usr/bin/x\n",
            "%post",
            ScriptletKind::Post,
        );
        let s = scriptlet(&sec);
        assert_eq!(s.body.lines.len(), 2);
    }
}
