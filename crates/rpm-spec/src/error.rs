//! Fatal error types for the parser and printer.
//!
//! Recoverable issues encountered during parsing are carried as
//! [`crate::parse_result::Diagnostic`] entries; the types in this module
//! represent failures that prevent producing any output at all.

#![allow(missing_docs)]

use crate::ast::Span;

/// A fatal parser failure.
///
/// The variants below cover the most common failure classes; the enum is
/// `#[non_exhaustive]` so downstream code must always include a wildcard
/// arm.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub enum ParseError {
    /// I/O failure while reading source bytes.
    #[error("I/O error: {message}")]
    Io { message: String },
    /// A token was rejected by the lexer / low-level parser.
    #[error("syntax error at {span:?}: {message}")]
    Syntax {
        span:    Option<Span>,
        message: String,
    },
    /// An `%if` / `%ifarch` / `%ifos` opened but never closed.
    #[error("unterminated conditional block opened at {opened_at:?}")]
    UnterminatedConditional { opened_at: Option<Span> },
    /// A section header was malformed (e.g. unknown name, missing argument).
    #[error("invalid section header at {span:?}: {message}")]
    InvalidSection {
        span:    Option<Span>,
        message: String,
    },
}

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
