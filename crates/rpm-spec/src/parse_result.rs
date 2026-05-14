//! Result type and diagnostics returned by the parser.

use crate::ast::{SpecFile, Span};

/// Outcome of parsing a `.spec` source.
///
/// The parser always returns a [`SpecFile`] (even if partial) along with a
/// list of recoverable issues. Fatal errors that prevent producing any AST
/// are signalled by [`crate::error::ParseError`] from the parser entry
/// points.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParseResult<T = ()> {
    pub spec:        SpecFile<T>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnostic {
    pub severity: Severity,
    pub span:     Option<Span>,
    pub message:  String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Severity {
    Warning,
    Error,
}
