//! Parser input type and span helpers.
//!
//! The parser consumes [`Input<'a>`], a `LocatedSpan<&'a str>` from
//! `nom_locate`. Spans are produced by [`span_between`] from a "before"
//! and an "after" input cursor — `before` is the position at the start of
//! the matched fragment, `after` is what nom returned as remaining input.

use nom_locate::LocatedSpan;

use crate::ast::Span;

/// Input fed to every parser combinator. The wrapper carries byte-offset
/// and 1-based line/column information for free.
pub type Input<'a> = LocatedSpan<&'a str>;

/// Build an [`Input`] over an entire source string.
pub fn input(source: &str) -> Input<'_> {
    Input::new(source)
}

/// Build a [`Span`] that covers the bytes consumed between `before` and
/// `after`, where both are nom-locate cursors into the same source.
///
/// The convention follows nom's normal flow: `before` is the input you
/// matched against; `after` is the input returned by the combinator (i.e.
/// what is still unconsumed). The resulting span is half-open
/// `[before, after)`.
pub fn span_between(before: &Input<'_>, after: &Input<'_>) -> Span {
    let start_byte = before.location_offset();
    let end_byte = after.location_offset();
    Span::new(
        start_byte,
        end_byte,
        before.location_line(),
        before.get_column() as u32,
        after.location_line(),
        after.get_column() as u32,
    )
}

/// Build a [`Span`] of zero length at the cursor position. Useful for
/// diagnostics that point at a position rather than a range.
pub fn span_at(cursor: &Input<'_>) -> Span {
    let byte = cursor.location_offset();
    let line = cursor.location_line();
    let col = cursor.get_column() as u32;
    Span::new(byte, byte, line, col, line, col)
}

/// Build a [`Span`] over a single line's text, **excluding** the
/// trailing line-ending bytes.
///
/// `start` is the cursor at the beginning of the line; `text` is the
/// `not_line_ending` capture (i.e. the line content without `\n` or
/// `\r\n`). We can't just use `span_between(start, after_line)` because
/// `after_line` sits on the *next* line — that would render in
/// `codespan` as a multi-line carat covering the unrelated line below.
pub fn span_for_line(start: &Input<'_>, text: &Input<'_>) -> Span {
    let start_byte = start.location_offset();
    let end_byte = text.location_offset() + text.fragment().len();
    let line = start.location_line();
    let start_col = start.get_column() as u32;
    let end_col = start_col + text.fragment().chars().count() as u32;
    Span::new(start_byte, end_byte, line, start_col, line, end_col)
}
