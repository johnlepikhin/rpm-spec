//! Parser and pretty-printer for RPM `.spec` files.
//!
//! This crate exposes a distribution-independent AST suitable for tooling
//! such as formatters, linters, and static analyzers. Macros are preserved
//! as AST nodes (never expanded), so consumers can inspect or rewrite the
//! source without losing structural information.
//!
//! # Status
//!
//! Pre-alpha. The AST is in place; the parser and printer are stubs.
//!
//! # Crate layout
//!
//! - [`ast`] — abstract syntax tree.
//! - [`parse_result`] — [`parse_result::ParseResult`] /
//!   [`parse_result::Diagnostic`] returned by the parser.
//! - [`parser`] — `&str → ParseResult` (feature `parser`, in progress).
//! - [`printer`] — `AST → String` (feature `printer`, in progress).
//! - [`error`] — fatal error types.
//!
//! # Generic `T` parameter
//!
//! Every "large" AST node carries a `data: T` field; the root is
//! [`ast::SpecFile<T>`]. `T` defaults to `()`. The parser populates it with
//! [`ast::Span`] when a span-aware entry point is used. Validators may
//! choose a richer type to thread their own per-node data (resolved macro
//! values, diagnostics ids, …).
//!
//! # Macro names are verbatim
//!
//! [`ast::MacroRef::name`], [`ast::MacroDef::name`],
//! [`ast::BuildCondition::name`], and the `Other` variants of [`ast::Tag`],
//! [`ast::TagQualifier`], and [`ast::BuiltinMacro`] preserve the exact text
//! written in the source — case is **not** normalized. This invariant exists
//! so that downstream validators can match names against
//! distribution-specific registries.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
#![deny(missing_docs)]

pub mod ast;
pub mod error;
pub mod parse_result;

#[cfg(feature = "parser")]
pub mod parser;

#[cfg(feature = "printer")]
pub mod printer;
