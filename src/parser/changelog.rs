//! Parser for `%changelog` sections.
//!
//! Entry header format:
//!
//! ```text
//! * Weekday Month Day Year Author [<email>] [- VERSION[-RELEASE]]
//! ```
//!
//! Body lines (everything between this header and the next `*`-headed
//! entry or section header) are stored as `Vec<Text>` with macros parsed.

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{ChangelogDate, ChangelogEntry, Month, Section, Span, Text, Weekday};
use crate::parse_result::codes;

use super::input::{Input, span_between};
use super::section::peek_section_header;
use super::state::ParserState;
use super::text::parse_body_as_text;
use super::util::{line_terminator, physical_line, space0};

/// Parse a `%changelog` section header and body into
/// [`Section::Changelog`].
pub fn parse_changelog_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%changelog".len());
    let (after_header, _) = line_terminator(after_kw)?;

    let (after_body, entries) = collect_entries(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Changelog {
            entries,
            data: span,
        },
    ))
}

fn collect_entries<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Vec<ChangelogEntry<Span>>) {
    let mut cursor = input;
    let mut entries: Vec<ChangelogEntry<Span>> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        // Skip blank lines between entries.
        let (after_ws, _) = match space0(cursor) {
            Ok(r) => r,
            Err(_) => break,
        };
        let frag = *after_ws.fragment();
        if frag.is_empty() {
            cursor = after_ws;
            break;
        }
        if !frag.starts_with('*') {
            // Unexpected text before the first `*` — consume one physical
            // line so we keep making progress.
            let here = cursor;
            let (after, _) = match physical_line(here) {
                Ok(r) => r,
                Err(_) => break,
            };
            if after.location_offset() == here.location_offset() {
                break;
            }
            // Only complain about non-blank stray lines.
            if !line_is_blank(here.fragment()) {
                state.push_warning_code(
                    codes::W_UNEXPECTED_LINE_IN_CHANGELOG,
                    "unexpected line outside a %changelog entry",
                    Some(span_between(&here, &after)),
                );
            }
            cursor = after;
            continue;
        }

        match parse_changelog_entry(state, cursor) {
            Ok((rest, entry)) => {
                if rest.location_offset() == cursor.location_offset() {
                    break;
                }
                entries.push(entry);
                cursor = rest;
            }
            Err(_) => {
                let here = cursor;
                let (after, _) = match physical_line(here) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                if after.location_offset() == here.location_offset() {
                    break;
                }
                state.push_warning_code(
                    codes::W_MALFORMED_CHANGELOG_HEADER,
                    "malformed %changelog entry header",
                    Some(span_between(&here, &after)),
                );
                cursor = after;
            }
        }
    }

    (cursor, entries)
}

fn line_is_blank(s: &str) -> bool {
    let line = s.split(['\n', '\r']).next().unwrap_or(s);
    line.trim().is_empty()
}

/// Parse one `%changelog` entry (header + body lines).
pub fn parse_changelog_entry<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, ChangelogEntry<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let frag = *after_ws.fragment();
    if !frag.starts_with('*') {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }
    // Strip the leading `*` + whitespace.
    let after_star =
        advance(after_ws, 1).ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;
    let (after_star_ws, _) = space0(after_star)?;

    let header_start = after_star_ws;
    let (after_header_line, header_input) = physical_line(after_star_ws)?;
    let header_span = span_between(&header_start, &after_header_line);

    let header_text = header_input.fragment();
    let (date, author, email, version) = parse_header_text(state, header_text, header_span)
        .ok_or_else(|| nom::Err::Error(error_position!(input, ErrorKind::Tag)))?;

    // Collect body lines.
    let mut cursor = after_header_line;
    let mut body: Vec<Text> = Vec::new();
    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        // Stop at the next entry header (line starting with `*` after ws).
        let (after_ws2, _) = space0(cursor)?;
        if after_ws2.fragment().starts_with('*') {
            break;
        }
        let here = cursor;
        let (after, line_input) = physical_line(here)?;
        if after.location_offset() == here.location_offset() {
            break;
        }
        body.push(parse_body_as_text(state, line_input.fragment()));
        cursor = after;
    }
    // Trim trailing empty body lines.
    while matches!(body.last(), Some(t) if text_is_empty(t)) {
        body.pop();
    }

    let span = span_between(&start, &cursor);
    Ok((
        cursor,
        ChangelogEntry {
            date,
            author,
            email,
            version,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// Header parsing helpers
// ---------------------------------------------------------------------

/// Lower bound of the plausible-year range for `%changelog` entries.
///
/// We trust the Unix epoch year (1970) as the floor: package
/// maintainers occasionally backdate entries, and rounding to 1970
/// keeps the rule easy to remember. Anything earlier is almost
/// certainly a typo.
const MIN_PLAUSIBLE_YEAR: u16 = 1970;
/// Upper bound of the plausible-year range; paired with
/// [`MIN_PLAUSIBLE_YEAR`]. Picked as a far-future sentinel — bump it
/// if RPM is still in use in the 23rd century.
const MAX_PLAUSIBLE_YEAR: u16 = 2200;

fn parse_header_text(
    state: &ParserState,
    header: &str,
    header_span: Span,
) -> Option<(ChangelogDate, Text, Option<Text>, Option<Text>)> {
    // Tokens: Weekday Month Day Year ...
    let mut tokens = header.split_whitespace();
    let weekday = parse_weekday(tokens.next()?)?;
    let month = parse_month(tokens.next()?)?;
    let day: u8 = tokens.next()?.parse().ok()?;
    let year: u16 = tokens.next()?.parse().ok()?;

    if !(1..=31).contains(&day) {
        state.push_warning_code(
            codes::W_IMPLAUSIBLE_CHANGELOG_DATE,
            format!("day-of-month `{day}` is out of range 1..=31"),
            Some(header_span),
        );
    }
    if !(MIN_PLAUSIBLE_YEAR..=MAX_PLAUSIBLE_YEAR).contains(&year) {
        state.push_warning_code(
            codes::W_IMPLAUSIBLE_CHANGELOG_DATE,
            format!(
                "year `{year}` is implausible (expected \
                 {MIN_PLAUSIBLE_YEAR}..={MAX_PLAUSIBLE_YEAR})"
            ),
            Some(header_span),
        );
    }

    let date = ChangelogDate {
        weekday,
        month,
        day,
        year,
    };

    // The remainder of `header` after the first four whitespace-separated
    // tokens is the author segment (with optional email and version).
    let rest = consume_first_four_tokens(header)?.trim();
    let (author_str, email_str, version_str) = split_author_email_version(rest);

    let author = parse_body_as_text(state, author_str.trim());
    let email = email_str.map(|e| parse_body_as_text(state, e.trim()));
    let version = version_str.map(|v| parse_body_as_text(state, v.trim()));

    Some((date, author, email, version))
}

fn consume_first_four_tokens(s: &str) -> Option<&str> {
    let mut idx = 0;
    let bytes = s.as_bytes();
    let mut tokens_consumed = 0;
    while idx < bytes.len() && tokens_consumed < 4 {
        // Skip whitespace.
        while idx < bytes.len() && matches!(bytes[idx], b' ' | b'\t') {
            idx += 1;
        }
        if idx >= bytes.len() {
            return None;
        }
        // Skip token.
        while idx < bytes.len() && !matches!(bytes[idx], b' ' | b'\t') {
            idx += 1;
        }
        tokens_consumed += 1;
    }
    if tokens_consumed < 4 {
        return None;
    }
    Some(&s[idx..])
}

/// Split the remainder of a changelog header into author, optional
/// `<email>`, and optional trailing `- VERSION`.
fn split_author_email_version(rest: &str) -> (&str, Option<&str>, Option<&str>) {
    // 1. Find " - " separator for version (last occurrence wins so the
    //    author may contain hyphens).
    let (head, version) = match rfind_dash_separator(rest) {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 3..])),
        None => (rest, None),
    };
    // 2. Inside head, find `<…>` for email.
    let (author, email) = match (head.find('<'), head.rfind('>')) {
        (Some(lt), Some(gt)) if gt > lt => {
            let author = head[..lt].trim_end();
            let email = &head[lt + 1..gt];
            (author, Some(email))
        }
        _ => (head.trim_end(), None),
    };
    (author, email, version)
}

/// Find the byte index of the rightmost ` - ` (space-dash-space) at the
/// top of `head`. Used to peel off `- VERSION` while tolerating hyphens
/// in author names.
fn rfind_dash_separator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let mut i = bytes.len() - 3;
    loop {
        if &bytes[i..i + 3] == b" - " {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    match s {
        "Mon" => Some(Weekday::Mon),
        "Tue" => Some(Weekday::Tue),
        "Wed" => Some(Weekday::Wed),
        "Thu" => Some(Weekday::Thu),
        "Fri" => Some(Weekday::Fri),
        "Sat" => Some(Weekday::Sat),
        "Sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_month(s: &str) -> Option<Month> {
    match s {
        "Jan" => Some(Month::Jan),
        "Feb" => Some(Month::Feb),
        "Mar" => Some(Month::Mar),
        "Apr" => Some(Month::Apr),
        "May" => Some(Month::May),
        "Jun" => Some(Month::Jun),
        "Jul" => Some(Month::Jul),
        "Aug" => Some(Month::Aug),
        "Sep" => Some(Month::Sep),
        "Oct" => Some(Month::Oct),
        "Nov" => Some(Month::Nov),
        "Dec" => Some(Month::Dec),
        _ => None,
    }
}

fn text_is_empty(t: &Text) -> bool {
    t.segments
        .iter()
        .all(|s| matches!(s, crate::ast::TextSegment::Literal(s) if s.trim().is_empty()))
}

fn advance<'a>(input: Input<'a>, n: usize) -> Option<Input<'a>> {
    if input.fragment().len() < n {
        return None;
    }
    let (rest, _) = nom::Input::take_split(&input, n);
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Section<Span> {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_changelog_section(&state, inp).unwrap();
        sec
    }

    fn entries(sec: &Section<Span>) -> &Vec<ChangelogEntry<Span>> {
        match sec {
            Section::Changelog { entries, .. } => entries,
            _ => panic!(),
        }
    }

    #[test]
    fn one_entry_with_version() {
        let src = "%changelog\n* Wed May 14 2025 Maintainer <m@example.org> - 1.0-1\n- initial packaging\n";
        let sec = parse(src);
        let es = entries(&sec);
        assert_eq!(es.len(), 1);
        let e = &es[0];
        assert_eq!(e.date.weekday, Weekday::Wed);
        assert_eq!(e.date.month, Month::May);
        assert_eq!(e.date.day, 14);
        assert_eq!(e.date.year, 2025);
        assert_eq!(e.author.literal_str(), Some("Maintainer"));
        assert_eq!(e.email.as_ref().unwrap().literal_str(), Some("m@example.org"));
        assert_eq!(e.version.as_ref().unwrap().literal_str(), Some("1.0-1"));
        assert_eq!(e.body.len(), 1);
        assert_eq!(e.body[0].literal_str(), Some("- initial packaging"));
    }

    #[test]
    fn entry_without_version() {
        let src = "%changelog\n* Mon Jan 01 2024 Alice <a@example.com>\n- something\n";
        let sec = parse(src);
        let es = entries(&sec);
        assert!(es[0].version.is_none());
        assert_eq!(es[0].email.as_ref().unwrap().literal_str(), Some("a@example.com"));
    }

    #[test]
    fn entry_without_email() {
        let src = "%changelog\n* Tue Feb 02 2024 Bob - 2.0-1\n- something\n";
        let sec = parse(src);
        let es = entries(&sec);
        assert!(es[0].email.is_none());
        assert_eq!(es[0].author.literal_str(), Some("Bob"));
        assert_eq!(es[0].version.as_ref().unwrap().literal_str(), Some("2.0-1"));
    }

    #[test]
    fn two_entries() {
        let src = "\
%changelog
* Wed May 14 2025 A <a@x.org> - 1.0-1
- newer
* Mon Jan 01 2024 B <b@x.org> - 0.9-1
- older
";
        let sec = parse(src);
        let es = entries(&sec);
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].date.year, 2025);
        assert_eq!(es[1].date.year, 2024);
    }

    #[test]
    fn entry_with_multiline_body() {
        let src = "\
%changelog
* Wed May 14 2025 A <a@x.org> - 1.0-1
- first
- second
- third
";
        let sec = parse(src);
        let es = entries(&sec);
        assert_eq!(es[0].body.len(), 3);
    }

    #[test]
    fn entry_with_macro_in_version() {
        let src = "%changelog\n* Wed May 14 2025 A <a@x.org> - 1.0-1%{?dist}\n- x\n";
        let sec = parse(src);
        let es = entries(&sec);
        let ver = es[0].version.as_ref().unwrap();
        // The version should contain the literal "1.0-1" plus a macro for dist.
        assert!(
            ver.segments
                .iter()
                .any(|s| matches!(s, crate::ast::TextSegment::Macro(m) if m.name == "dist"))
        );
    }

    #[test]
    fn changelog_stops_at_next_section_header() {
        let src = "%changelog\n* Wed May 14 2025 A <a@x.org> - 1.0-1\n- body\n%files\n";
        let sec = parse(src);
        let es = entries(&sec);
        assert_eq!(es.len(), 1);
        // Body should not contain "%files".
        for line in &es[0].body {
            assert!(!line.literal_str().unwrap_or("").contains("%files"));
        }
    }

    #[test]
    fn author_with_hyphen() {
        // Author name itself contains a hyphen; version must still be peeled.
        let src = "%changelog\n* Wed May 14 2025 Foo-Bar Baz <fb@example.org> - 1.0-1\n- x\n";
        let sec = parse(src);
        let es = entries(&sec);
        assert_eq!(es[0].author.literal_str(), Some("Foo-Bar Baz"));
        assert_eq!(es[0].version.as_ref().unwrap().literal_str(), Some("1.0-1"));
    }
}
