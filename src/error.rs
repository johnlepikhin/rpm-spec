//! Fatal error types for the parser and printer.
//!
//! Recoverable issues encountered during parsing are carried as
//! [`crate::parse_result::Diagnostic`] entries; the types in this module
//! represent failures that prevent producing any output at all.

#![allow(missing_docs)]

use crate::ast::Span;

/// A fatal parser failure.
///
/// The current parser entry points (`parse_str`, `parse_str_with_spans`)
/// never produce a [`ParseError`]: they always return a partial
/// [`crate::ast::SpecFile`] along with a list of
/// [`crate::parse_result::Diagnostic`]. This enum is reserved for
/// future entry points that read from `io::Read` / files, where I/O
/// failure is a genuinely fatal condition that cannot be modelled as a
/// diagnostic.
///
/// The enum is `#[non_exhaustive]` so adding new variants is non-breaking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub enum ParseError {
    /// I/O failure while reading source bytes. Reserved for future
    /// `parse_reader` / `parse_file` entry points.
    // TODO(parser-TODO.md Stage 6): when `parse_reader`/`parse_file`
    // land, carry richer I/O context here so callers can branch on
    // `NotFound` vs `PermissionDenied` without string-matching. Note
    // that the current `Clone + Eq` derives will need to be revisited
    // (`std::io::Error` is neither).
    #[error("I/O error: {message}")]
    Io { message: String },
}

// Placeholder usage to keep `Span` reachable for future variants.
#[allow(dead_code)]
const _: Option<Span> = None;

/// A fatal printer failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub enum PrintError {
    /// The pretty-printer's internal layout engine failed.
    #[error("layout error: {message}")]
    Layout { message: String },
    /// `std::fmt::Write` propagated an error from the underlying sink.
    #[error("write error: {message}")]
    Write { message: String },
}
