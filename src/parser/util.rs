//! Lexical helpers shared by every sub-parser.

#![allow(missing_docs)]

use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_while},
    character::complete::{line_ending, not_line_ending},
    combinator::{opt, recognize, value},
    multi::many0,
    sequence::{pair, preceded, terminated},
};

use super::input::Input;

/// UTF-8 byte-order mark.
pub const BOM: &str = "\u{feff}";

/// Strip a leading UTF-8 BOM if present, leaving the rest untouched.
pub fn strip_bom<'a>(input: Input<'a>) -> Input<'a> {
    match tag::<_, _, ()>(BOM).parse(input) {
        Ok((rest, _)) => rest,
        Err(_) => input,
    }
}

/// Consume zero or more space/tab characters (no line endings).
pub fn space0<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    take_while(is_space_or_tab).parse(input)
}

/// Consume one or more space/tab characters (no line endings).
pub fn space1<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    nom::bytes::complete::take_while1(is_space_or_tab).parse(input)
}

#[inline]
pub fn is_space_or_tab(c: char) -> bool {
    c == ' ' || c == '\t'
}

#[inline]
pub fn is_macro_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[inline]
pub fn is_macro_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Match a single line ending (`\n` or `\r\n`).
pub fn eol<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    line_ending.parse(input)
}

/// Consume a "physical" line: everything up to and including the next line
/// ending (or end of input). Returns the text *without* the line ending.
pub fn physical_line<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    terminated(not_line_ending, opt(line_ending)).parse(input)
}

/// Consume a *logical* line that may span several physical lines via
/// trailing `\`-continuations.
///
/// Returns the concatenated content with `\n` separators preserved between
/// joined lines, and the backslashes stripped. The returned `Input` is the
/// position immediately after the last consumed physical line.
pub fn logical_line<'a>(input: Input<'a>) -> IResult<Input<'a>, String> {
    let (mut rest, first) = continued_line_segment(input)?;
    let mut joined: String = trim_trailing_backslash(first.fragment()).to_owned();
    let mut last_had_continuation = ends_with_backslash(first.fragment());

    while last_had_continuation {
        let (next_rest, next) = match continued_line_segment(rest) {
            Ok(r) => r,
            Err(_) => break,
        };
        joined.push('\n');
        joined.push_str(trim_trailing_backslash(next.fragment()));
        rest = next_rest;
        last_had_continuation = ends_with_backslash(next.fragment());
    }

    Ok((rest, joined))
}

fn continued_line_segment<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    terminated(not_line_ending, opt(line_ending)).parse(input)
}

fn ends_with_backslash(s: &str) -> bool {
    // A line ends with a continuation if its last non-CR character is `\`.
    let trimmed = s.trim_end_matches('\r');
    trimmed.ends_with('\\')
}

fn trim_trailing_backslash(s: &str) -> &str {
    let trimmed = s.trim_end_matches('\r');
    if let Some(stripped) = trimmed.strip_suffix('\\') {
        stripped.trim_end_matches(is_space_or_tab)
    } else {
        trimmed
    }
}

/// Consume one blank line (whitespace + line ending or EOF after
/// whitespace).
pub fn blank_line<'a>(input: Input<'a>) -> IResult<Input<'a>, Input<'a>> {
    recognize(pair(
        space0,
        alt((value((), line_ending), value((), nom::combinator::eof))),
    ))
    .parse(input)
}

/// Skip any number of blank lines.
pub fn skip_blank_lines<'a>(input: Input<'a>) -> IResult<Input<'a>, ()> {
    let (rest, _) = many0(blank_line).parse(input)?;
    Ok((rest, ()))
}

/// Consume `space*` then a newline (or EOF). Used to round off a parsed
/// directive.
pub fn line_terminator<'a>(input: Input<'a>) -> IResult<Input<'a>, ()> {
    let (rest, _) = preceded(
        space0,
        alt((value((), line_ending), value((), nom::combinator::eof))),
    )
    .parse(input)?;
    Ok((rest, ()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bom_works() {
        let i = Input::new("\u{feff}hello");
        let stripped = strip_bom(i);
        assert_eq!(*stripped.fragment(), "hello");
    }

    #[test]
    fn strip_bom_idempotent_without_bom() {
        let i = Input::new("hello");
        let stripped = strip_bom(i);
        assert_eq!(*stripped.fragment(), "hello");
    }

    #[test]
    fn logical_line_no_continuation() {
        let i = Input::new("hello\nworld\n");
        let (rest, line) = logical_line(i).unwrap();
        assert_eq!(line, "hello");
        assert_eq!(*rest.fragment(), "world\n");
    }

    #[test]
    fn logical_line_with_continuation() {
        let i = Input::new("first \\\nsecond\nthird\n");
        let (rest, line) = logical_line(i).unwrap();
        assert_eq!(line, "first\nsecond");
        assert_eq!(*rest.fragment(), "third\n");
    }

    #[test]
    fn logical_line_three_continuations() {
        let i = Input::new("a\\\nb\\\nc\\\nd\nrest\n");
        let (rest, line) = logical_line(i).unwrap();
        assert_eq!(line, "a\nb\nc\nd");
        assert_eq!(*rest.fragment(), "rest\n");
    }

    #[test]
    fn blank_line_matches_empty() {
        let i = Input::new("\nnext\n");
        let (rest, _) = blank_line(i).unwrap();
        assert_eq!(*rest.fragment(), "next\n");
    }

    #[test]
    fn blank_line_matches_whitespace_only() {
        let i = Input::new("   \nnext\n");
        let (rest, _) = blank_line(i).unwrap();
        assert_eq!(*rest.fragment(), "next\n");
    }
}
