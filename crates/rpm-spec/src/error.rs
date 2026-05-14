//! Fatal error types for the parser and printer.
//!
//! Recoverable issues encountered during parsing are carried as
//! [`crate::parse_result::Diagnostic`] entries; the types in this module
//! represent failures that prevent producing any output at all.

use core::fmt;

/// A fatal parser failure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("parse error")
    }
}

impl std::error::Error for ParseError {}

/// A fatal printer failure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PrintError;

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("print error")
    }
}

impl std::error::Error for PrintError {}
