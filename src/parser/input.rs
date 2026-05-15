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
