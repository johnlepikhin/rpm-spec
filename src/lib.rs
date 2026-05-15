//! Parser and pretty-printer for RPM `.spec` files.
//!
//! This crate exposes a distribution-independent AST suitable for tooling
//! such as formatters, linters, and static analyzers. Macros are preserved
//! as AST nodes (never expanded), so consumers can inspect or rewrite the
//! source without losing structural information.
//!
//! # Overview
//!
//! - **AST modelling.** A typed, distribution-independent tree rooted at
//!   [`ast::SpecFile<T>`] that covers preamble lines, sections, conditional
//!   blocks, macro definitions, `%bcond*` toggles, rich/boolean dependency
//!   expressions, structured `%if` / `%elif` expression ASTs
//!   ([`ast::CondExpr`], [`ast::ExprAst`]), comments and blank lines.
//! - **Span-aware parsing.** A recovery-oriented parser that never panics on
//!   real input. Both [`parser::parse_str`] and
//!   [`parser::parse_str_with_spans`] always return a (possibly partial)
//!   [`parse_result::ParseResult`] with a [`ast::SpecFile`] plus a
//!   `Vec<`[`parse_result::Diagnostic`]`>`. Recoverable issues are surfaced
//!   as diagnostics with stable identifiers from
//!   [`parse_result::codes`] (`rpmspec/E####` for errors,
//!   `rpmspec/W####` for warnings) that are stable across patch releases.
//! - **Pretty-printer.** [`printer::print`] / [`printer::print_with`]
//!   re-emit an `&`[`ast::SpecFile<T>`] as a normalised but structurally
//!   equivalent source. The classified-token API
//!   ([`printer::PrintWriter`], [`printer::TokenKind`], [`printer::print_to`])
//!   exposes each emitted chunk together with its source-level category, so
//!   syntax highlighters and rich editors can consume printer output
//!   directly without re-tokenising.
//!
//! # Crate layout
//!
//! - [`ast`] — abstract syntax tree.
//! - [`parse_result`] — [`parse_result::ParseResult`] /
//!   [`parse_result::Diagnostic`] returned by the parser, plus stable
//!   diagnostic [`parse_result::codes`].
//! - [`parser`] — `&str → ParseResult` (feature `parser`).
//! - [`printer`] — `AST → String` and the classified-token writer
//!   (feature `printer`).
//! - [`error`] — fatal error types, reserved for future I/O-based entry
//!   points.
//!
//! # Cargo features
//!
//! The default feature set is `["parser", "printer"]`. Every feature is
//! additive.
//!
//! | Feature   | Default | Effect                                                                         |
//! | --------- | ------- | ------------------------------------------------------------------------------ |
//! | `parser`  | yes     | Compiles the [`parser`] module and its entry points. Pulls in `nom`.           |
//! | `printer` | yes     | Compiles the [`printer`] module. No extra dependencies.                        |
//! | `serde`   | no      | Derives `Serialize` / `Deserialize` on the AST, diagnostics and config types.  |
//! | `tracing` | no      | Adds `#[tracing::instrument]` on hot-path parser entry points.                 |
//!
//! # Quick start
//!
//! Parse a spec, inspect diagnostics, and round-trip back to source:
//!
//! ```
//! use rpm_spec::{parser, printer};
//!
//! let src = "Name:           foo\nVersion:        1.0\n";
//! let result = parser::parse_str_with_spans(src);
//! assert!(result.diagnostics.is_empty());
//!
//! let printed = printer::print(&result.spec);
//! assert!(printed.contains("Name:"));
//! ```
//!
//! # Generic `T` parameter
//!
//! Every "large" AST node carries a `data: T` field; the root is
//! [`ast::SpecFile<T>`]. `T` defaults to `()`. [`parser::parse_str`] returns
//! [`parse_result::ParseResult`]`<()>`, while
//! [`parser::parse_str_with_spans`] populates `T` with [`ast::Span`]
//! (byte offset plus 1-based line and column at both ends). Validators may
//! choose a richer type to thread their own per-node data (resolved macro
//! values, validator diagnostic ids, …) and map between representations.
//!
//! # Macro names are verbatim
//!
//! [`ast::MacroRef::name`], [`ast::MacroDef::name`],
//! [`ast::BuildCondition::name`], and the `Other` variants of [`ast::Tag`],
//! [`ast::TagQualifier`], and [`ast::BuiltinMacro`] preserve the exact text
//! written in the source — case is **not** normalised. This invariant exists
//! so that downstream validators can match names against
//! distribution-specific registries.
//!
//! # Crate-level lints
//!
//! The crate is `#![forbid(unsafe_code)]` (no `unsafe` blocks anywhere) and
//! `#![deny(missing_docs)]` (every public item must be documented).

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
