//! Parser for `%files` sections.
//!
//! Body grammar (one physical line per [`FileEntry`]):
//!
//! ```text
//! file-line := directive* path?
//! directive := '%attr' '(' attr-fields ')'
//!            | '%defattr' '(' defattr-fields ')'
//!            | '%dir'
//!            | '%doc'
//!            | '%license'
//!            | '%config' [ '(' config-flags ')' ]
//!            | '%ghost'
//!            | '%verify' '(' [ 'not' ] verify-checks ')'
//!            | '%lang' '(' locale ')'
//!            | '%caps' '(' caps-spec ')'
//!            | '%artifact'
//!            | '%missingok'
//! ```
//!
//! `%if`/`%endif` blocks inside `%files` are parsed structurally as
//! [`FilesContent::Conditional`] via the generic [`super::cond::parse_conditional`].

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{
    AttrField, AttrFields, ConfigFlag, DefattrFields, FileDirective, FileEntry, FilePath,
    FilesContent, Section, Span, SpecItem, SubpkgRef, Text, VerifyCheck,
};
use crate::parse_result::codes;

use super::cond::parse_conditional;
use super::input::{Input, span_at, span_between, span_for_line};
use super::macros::parse_hash_comment;
use super::section::{peek_section_header, take_name_with_macros};
use super::state::ParserState;
use super::text::parse_body_as_text;
use super::util::{blank_line, line_terminator, physical_line, space0};

const FILE_LIST_FLAG: &str = "-f";

/// Whitelist of file directive names. `%xyz` where `xyz` is *not* in
/// this set is treated as the start of the path (a macro reference).
const DIRECTIVE_NAMES: &[&str] = &[
    "%defattr",
    "%attr",
    "%dir",
    "%doc",
    "%license",
    "%config",
    "%ghost",
    "%verify",
    "%lang",
    "%caps",
    "%artifact",
    "%missingok",
];

/// Parse a `%files` section header and body into [`Section::Files`].
pub fn parse_files_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%files".len());
    let (after_header, (subpkg, file_lists)) = parse_files_header(state, after_kw)?;

    let (after_body, content) = collect_files_body(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Files {
            subpkg,
            file_lists,
            content,
            data: span,
        },
    ))
}

/// Body-item parser passed into `parse_conditional` so `%if` blocks
/// inside `%files` can nest.
pub fn parse_files_content<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Vec<FilesContent<Span>>> {
    // Blank line.
    if let Ok((rest, _)) = blank_line(input) {
        if rest.location_offset() > input.location_offset() {
            return Ok((rest, vec![FilesContent::Blank]));
        }
    }
    // `#` comment.
    let after_ws = match space0(input) {
        Ok((r, _)) => r,
        Err(_) => input,
    };
    if after_ws.fragment().starts_with('#') {
        if let Ok((rest, SpecItem::Comment(c))) = parse_hash_comment(state, input) {
            return Ok((rest, vec![FilesContent::Comment(c)]));
        }
    }
    // Nested conditional.
    if let Ok((rest, c)) = parse_conditional(state, input, parse_files_content) {
        return Ok((rest, vec![FilesContent::Conditional(c)]));
    }
    // File entry.
    let (rest, entry) = parse_file_entry(state, input)?;
    Ok((rest, vec![FilesContent::Entry(entry)]))
}

fn collect_files_body<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Vec<FilesContent<Span>>) {
    let mut cursor = input;
    let mut content: Vec<FilesContent<Span>> = Vec::new();
    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        match parse_files_content(state, cursor) {
            Ok((rest, items)) => {
                if rest.location_offset() == cursor.location_offset() {
                    break;
                }
                content.extend(items);
                cursor = rest;
            }
            Err(_) => {
                let here = cursor;
                let (after, line_text) = match physical_line(here) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                if after.location_offset() == here.location_offset() {
                    break;
                }
                state.push_warning_code(
                    codes::W_LINE_NOT_RECOGNIZED_IN_FILES,
                    "line not recognized inside %files body",
                    Some(span_for_line(&here, &line_text)),
                );
                cursor = after;
            }
        }
    }
    (cursor, content)
}

// ---------------------------------------------------------------------
// %files header (`-f filelist` repeatable, optional subpkg)
// ---------------------------------------------------------------------

fn parse_files_header<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, (Option<SubpkgRef>, Vec<Text>)> {
    let mut cursor = input;
    let mut subpkg: Option<SubpkgRef> = None;
    let mut file_lists: Vec<Text> = Vec::new();

    #[allow(clippy::while_let_loop)] // body breaks on many branches
    loop {
        let (after_ws, _) = match space0(cursor) {
            Ok(r) => r,
            Err(_) => break,
        };
        cursor = after_ws;
        let frag = *cursor.fragment();
        if frag.is_empty() || frag.starts_with('\n') || frag.starts_with('\r') {
            break;
        }
        if frag.starts_with("-n") {
            cursor = match advance(cursor, 2) {
                Some(c) => c,
                None => break,
            };
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_name_with_macros(state, cursor) {
                Some((after_name, name)) => {
                    subpkg = Some(SubpkgRef::Absolute(name));
                    cursor = after_name;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_NAME_AFTER_N,
                        "%files: expected name after -n",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if frag.starts_with(FILE_LIST_FLAG) {
            cursor = match advance(cursor, FILE_LIST_FLAG.len()) {
                Some(c) => c,
                None => break,
            };
            let (after_ws2, _) = space0(cursor)?;
            cursor = after_ws2;
            match take_path_token(cursor) {
                Some((after_path, path)) => {
                    file_lists.push(parse_body_as_text(state, path));
                    cursor = after_path;
                }
                None => {
                    state.push_warning_code(
                        codes::W_EXPECTED_FILELIST,
                        "%files: expected filelist path after -f",
                        Some(span_at(&cursor)),
                    );
                    break;
                }
            }
        } else if subpkg.is_none() {
            // Bare subpackage name (relative form).
            match take_name_with_macros(state, cursor) {
                Some((after_name, name)) => {
                    subpkg = Some(SubpkgRef::Relative(name));
                    cursor = after_name;
                }
                None => break,
            }
        } else {
            break;
        }
    }

    let (after_term, _) = line_terminator(cursor)?;
    Ok((after_term, (subpkg, file_lists)))
}

// ---------------------------------------------------------------------
// One %files entry: directives + optional path
// ---------------------------------------------------------------------

fn parse_file_entry<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, FileEntry<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let mut cursor = after_ws;
    let mut directives: Vec<FileDirective> = Vec::new();

    loop {
        let frag = *cursor.fragment();
        if !frag.starts_with('%') {
            break;
        }
        let Some(dir_kw) = match_directive_keyword(frag) else {
            // `%xyz` not in whitelist — start of path (macro reference).
            break;
        };
        let after_kw = advance(cursor, dir_kw.len()).expect("matched length");
        let (after_directive, directive) = parse_directive_args(state, after_kw, dir_kw)?;
        directives.push(directive);
        let (after_ws2, _) = space0(after_directive)?;
        cursor = after_ws2;
    }

    // Remaining content of the line (up to EOL) is the file path.
    let path_input = cursor;
    let (after_line, line_content) = match physical_line(path_input) {
        Ok(r) => r,
        Err(_) => {
            return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
        }
    };
    let path_text = line_content.fragment().trim();
    let path = if path_text.is_empty() {
        None
    } else {
        Some(FilePath {
            path: parse_body_as_text(state, path_text),
        })
    };

    if directives.is_empty() && path.is_none() {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }

    let span = span_between(&start, &after_line);
    Ok((
        after_line,
        FileEntry {
            directives,
            path,
            data: span,
        },
    ))
}

fn match_directive_keyword(frag: &str) -> Option<&'static str> {
    for &kw in DIRECTIVE_NAMES {
        if frag.starts_with(kw) {
            // Must be followed by '(' or whitespace/EOL — otherwise it
            // is a longer macro name like `%docdir`.
            match frag.as_bytes().get(kw.len()).copied() {
                Some(b'(') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None => {
                    return Some(kw);
                }
                _ => continue,
            }
        }
    }
    None
}

fn parse_directive_args<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &'static str,
) -> IResult<Input<'a>, FileDirective> {
    match keyword {
        "%attr" => {
            let (rest, fields) = parse_paren_list_three(input)?;
            Ok((
                rest,
                FileDirective::Attr(Box::new(AttrFields {
                    mode: parse_attr_field(state, &fields[0]),
                    user: parse_attr_field(state, &fields[1]),
                    group: parse_attr_field(state, &fields[2]),
                })),
            ))
        }
        "%defattr" => {
            let (rest, fields) = parse_paren_list_three_or_four(input)?;
            let dmode = fields.get(3).map(|s| parse_attr_field(state, s));
            Ok((
                rest,
                FileDirective::Defattr(Box::new(DefattrFields {
                    fmode: parse_attr_field(state, &fields[0]),
                    user: parse_attr_field(state, &fields[1]),
                    group: parse_attr_field(state, &fields[2]),
                    dmode,
                })),
            ))
        }
        "%dir" => Ok((input, FileDirective::Dir)),
        "%doc" => Ok((input, FileDirective::Doc)),
        "%license" => Ok((input, FileDirective::License)),
        "%config" => {
            let frag = *input.fragment();
            if frag.starts_with('(') {
                let (rest, inner) = take_balanced_parens(input)?;
                let flags = parse_config_flags(&inner);
                Ok((rest, FileDirective::Config(flags)))
            } else {
                Ok((input, FileDirective::Config(Vec::new())))
            }
        }
        "%ghost" => Ok((input, FileDirective::Ghost)),
        "%verify" => {
            let (rest, inner) = take_balanced_parens(input)?;
            let (negate, checks) = parse_verify_args(&inner);
            Ok((rest, FileDirective::Verify { negate, checks }))
        }
        "%lang" => {
            let (rest, inner) = take_balanced_parens(input)?;
            Ok((
                rest,
                FileDirective::Lang(parse_body_as_text(state, inner.trim())),
            ))
        }
        "%caps" => {
            let (rest, inner) = take_balanced_parens(input)?;
            Ok((
                rest,
                FileDirective::Caps(parse_body_as_text(state, inner.trim())),
            ))
        }
        "%artifact" => Ok((input, FileDirective::Artifact)),
        "%missingok" => Ok((input, FileDirective::MissingOk)),
        _ => unreachable!("directive keyword not in whitelist: {keyword}"),
    }
}

fn parse_attr_field(state: &ParserState, raw: &str) -> AttrField {
    const MAX_FILE_MODE: u32 = 0o7777;
    let trimmed = raw.trim();
    if trimmed == "-" {
        return AttrField::Default;
    }
    // `%attr` / `%defattr` modes are written in octal. Only digits 0..=7
    // are valid; any 8 or 9 in the token means this is not a numeric
    // mode (likely a user/group name like "user8").
    if !trimmed.is_empty() && trimmed.bytes().all(|b| matches!(b, b'0'..=b'7')) {
        if let Ok(n) = u32::from_str_radix(trimmed, 8) {
            if n > MAX_FILE_MODE {
                state.push_warning_code(
                    codes::W_INVALID_NUMBER,
                    format!("file mode `{trimmed}` exceeds 0o7777"),
                    None,
                );
            }
            return AttrField::Numeric(n);
        }
    }
    AttrField::Name(parse_body_as_text(state, trimmed))
}

fn parse_config_flags(inner: &str) -> Vec<ConfigFlag> {
    inner
        .split(',')
        .filter_map(|p| match p.trim().to_ascii_lowercase().as_str() {
            "noreplace" => Some(ConfigFlag::NoReplace),
            "missingok" => Some(ConfigFlag::MissingOk),
            _ => None, // unknown flag or empty — silently dropped; validator's job
        })
        .collect()
}

fn parse_verify_args(inner: &str) -> (bool, Vec<VerifyCheck>) {
    let mut tokens = inner.split_whitespace();
    let mut negate = false;
    let first = tokens.next();
    let mut checks: Vec<VerifyCheck> = Vec::new();

    let parse_one = |tok: &str| -> Option<VerifyCheck> {
        match tok.to_ascii_lowercase().as_str() {
            "md5" => Some(VerifyCheck::Md5),
            "filedigest" => Some(VerifyCheck::FileDigest),
            "size" => Some(VerifyCheck::Size),
            "link" => Some(VerifyCheck::Link),
            "user" => Some(VerifyCheck::User),
            "group" => Some(VerifyCheck::Group),
            "mtime" => Some(VerifyCheck::Mtime),
            "mode" => Some(VerifyCheck::Mode),
            "rdev" => Some(VerifyCheck::Rdev),
            "caps" => Some(VerifyCheck::Caps),
            _ => None,
        }
    };

    if let Some(tok) = first {
        if tok.eq_ignore_ascii_case("not") {
            negate = true;
        } else if let Some(c) = parse_one(tok) {
            checks.push(c);
        }
    }
    for tok in tokens {
        if let Some(c) = parse_one(tok) {
            checks.push(c);
        }
    }
    (negate, checks)
}

// ---------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------

fn take_balanced_parens<'a>(input: Input<'a>) -> IResult<Input<'a>, String> {
    let frag = *input.fragment();
    if !frag.starts_with('(') {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }
    let bytes = frag.as_bytes();
    let mut depth: i32 = 0;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;
    let inner = frag[1..end - 1].to_owned();
    let (rest, _) = nom::Input::take_split(&input, end);
    Ok((rest, inner))
}

fn parse_paren_list_three<'a>(input: Input<'a>) -> IResult<Input<'a>, [String; 3]> {
    let (rest, inner) = take_balanced_parens(input)?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Count)));
    }
    Ok((
        rest,
        [
            parts[0].to_owned(),
            parts[1].to_owned(),
            parts[2].to_owned(),
        ],
    ))
}

fn parse_paren_list_three_or_four<'a>(input: Input<'a>) -> IResult<Input<'a>, Vec<String>> {
    let (rest, inner) = take_balanced_parens(input)?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Count)));
    }
    Ok((rest, parts.into_iter().map(str::to_owned).collect()))
}

fn advance<'a>(input: Input<'a>, n: usize) -> Option<Input<'a>> {
    if input.fragment().len() < n {
        return None;
    }
    let (rest, _) = nom::Input::take_split(&input, n);
    Some(rest)
}

fn take_path_token<'a>(input: Input<'a>) -> Option<(Input<'a>, &'a str)> {
    let frag = *input.fragment();
    let mut iter = frag.char_indices();
    let (_, first) = iter.next()?;
    if first == ' ' || first == '\t' || first == '\n' || first == '\r' {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in iter {
        if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
            break;
        }
        end = i + c.len_utf8();
    }
    let (rest, _) = nom::Input::take_split(&input, end);
    Some((rest, &frag[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Section as AstSection, TextSegment};

    fn parse_files(src: &str) -> Section<Span> {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_files_section(&state, inp).unwrap();
        sec
    }

    fn entries(sec: &Section<Span>) -> &Vec<FilesContent<Span>> {
        match sec {
            AstSection::Files { content, .. } => content,
            _ => panic!("expected Files section"),
        }
    }

    fn first_entry(sec: &Section<Span>) -> &FileEntry<Span> {
        for c in entries(sec) {
            if let FilesContent::Entry(e) = c {
                return e;
            }
        }
        panic!("no file entry");
    }

    #[test]
    fn header_no_args_with_path() {
        let s = parse_files("%files\n/usr/bin/hello\n");
        let e = first_entry(&s);
        assert!(e.directives.is_empty());
        assert_eq!(
            e.path.as_ref().unwrap().path.literal_str(),
            Some("/usr/bin/hello")
        );
    }

    #[test]
    fn header_with_subpkg_relative() {
        let s = parse_files("%files devel\n/usr/include/foo.h\n");
        match s {
            AstSection::Files {
                subpkg: Some(SubpkgRef::Relative(t)),
                ..
            } => {
                assert_eq!(t.literal_str(), Some("devel"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn header_with_subpkg_absolute() {
        let s = parse_files("%files -n libfoo\n/usr/lib/libfoo.so\n");
        match s {
            AstSection::Files {
                subpkg: Some(SubpkgRef::Absolute(t)),
                ..
            } => {
                assert_eq!(t.literal_str(), Some("libfoo"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn header_with_filelist() {
        let s = parse_files("%files -f files.list\n");
        match s {
            AstSection::Files { file_lists, .. } => {
                assert_eq!(file_lists.len(), 1);
                assert_eq!(file_lists[0].literal_str(), Some("files.list"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn header_with_two_filelists() {
        let s = parse_files("%files -f a.list -f b.list\n");
        match s {
            AstSection::Files { file_lists, .. } => assert_eq!(file_lists.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn attr_directive() {
        let s = parse_files("%files\n%attr(0755,root,root) /usr/bin/hello\n");
        let e = first_entry(&s);
        assert_eq!(e.directives.len(), 1);
        match &e.directives[0] {
            FileDirective::Attr(f) => {
                assert!(matches!(f.mode, AttrField::Numeric(0o755)));
                match &f.user {
                    AttrField::Name(t) => assert_eq!(t.literal_str(), Some("root")),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn defattr_with_dash() {
        let s = parse_files("%files\n%defattr(-,root,root,-)\n");
        let e = first_entry(&s);
        match &e.directives[0] {
            FileDirective::Defattr(f) => {
                assert!(matches!(f.fmode, AttrField::Default));
                assert!(matches!(f.dmode, Some(AttrField::Default)));
                match &f.user {
                    AttrField::Name(t) => assert_eq!(t.literal_str(), Some("root")),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
        assert!(e.path.is_none());
    }

    #[test]
    fn doc_directive() {
        let s = parse_files("%files\n%doc README.md\n");
        let e = first_entry(&s);
        assert!(matches!(e.directives[0], FileDirective::Doc));
        assert_eq!(
            e.path.as_ref().unwrap().path.literal_str(),
            Some("README.md")
        );
    }

    #[test]
    fn config_with_noreplace() {
        let s = parse_files("%files\n%config(noreplace) /etc/foo.conf\n");
        let e = first_entry(&s);
        match &e.directives[0] {
            FileDirective::Config(flags) => {
                assert_eq!(flags, &vec![ConfigFlag::NoReplace]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn config_bare() {
        let s = parse_files("%files\n%config /etc/bar.conf\n");
        let e = first_entry(&s);
        match &e.directives[0] {
            FileDirective::Config(flags) => assert!(flags.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn verify_with_not() {
        let s = parse_files("%files\n%verify(not md5 size mtime) /usr/bin/foo\n");
        let e = first_entry(&s);
        match &e.directives[0] {
            FileDirective::Verify { negate, checks } => {
                assert!(*negate);
                assert_eq!(
                    *checks,
                    vec![VerifyCheck::Md5, VerifyCheck::Size, VerifyCheck::Mtime]
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn lang_caps_artifact_missingok() {
        let s = parse_files(
            "%files\n%lang(ru) /usr/share/locale/ru/foo.mo\n%caps(cap_net_bind_service=ep) /usr/bin/foo\n%artifact /tmp/foo\n%missingok /etc/maybe\n",
        );
        let kinds: Vec<&FileDirective> = entries(&s)
            .iter()
            .filter_map(|c| match c {
                FilesContent::Entry(e) => e.directives.first(),
                _ => None,
            })
            .collect();
        assert!(matches!(kinds[0], FileDirective::Lang(_)));
        assert!(matches!(kinds[1], FileDirective::Caps(_)));
        assert!(matches!(kinds[2], FileDirective::Artifact));
        assert!(matches!(kinds[3], FileDirective::MissingOk));
    }

    #[test]
    fn multiple_directives_on_one_line() {
        let s = parse_files("%files\n%attr(0644,root,root) %config(noreplace) /etc/foo.conf\n");
        let e = first_entry(&s);
        assert_eq!(e.directives.len(), 2);
        assert!(matches!(e.directives[0], FileDirective::Attr(_)));
        assert!(matches!(e.directives[1], FileDirective::Config(_)));
    }

    #[test]
    fn path_with_macro() {
        let s = parse_files("%files\n%{_bindir}/hello\n");
        let e = first_entry(&s);
        assert!(e.directives.is_empty());
        let path = &e.path.as_ref().unwrap().path;
        assert_eq!(path.segments.len(), 2);
        assert!(matches!(&path.segments[0], TextSegment::Macro(m) if m.name == "_bindir"));
    }

    #[test]
    fn conditional_inside_files() {
        let src = "\
%files\n\
/usr/bin/always\n\
%if 0%{?fedora}\n\
/usr/bin/fedora-only\n\
%endif\n\
";
        let s = parse_files(src);
        let cs = entries(&s);
        let cond_count = cs
            .iter()
            .filter(|c| matches!(c, FilesContent::Conditional(_)))
            .count();
        assert_eq!(cond_count, 1);
    }

    #[test]
    fn comments_and_blank_lines() {
        let src = "%files\n# pre comment\n\n/usr/bin/hello\n";
        let s = parse_files(src);
        let cs = entries(&s);
        assert!(cs.iter().any(|c| matches!(c, FilesContent::Comment(_))));
        assert!(cs.iter().any(|c| matches!(c, FilesContent::Blank)));
        assert!(cs.iter().any(|c| matches!(c, FilesContent::Entry(_))));
    }
}
