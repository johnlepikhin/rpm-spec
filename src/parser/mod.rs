//! RPM `.spec` parser.
//!
//! See [`parse_str`] and [`parse_str_with_spans`] for the public entry
//! points. The parser is recovery-oriented: it accumulates
//! [`crate::parse_result::Diagnostic`] entries and continues at the next
//! synchronization point (top-level `%name` header or `Tag:` line) rather
//! than failing on the first error.
//!
//! Stage 1 (current): top-level macro statements, `%define` /
//! `%global` / `%undefine`, `%bcond*`, `%include`, `%dnl` /
//! `#` comments, and `%if` / `%ifarch` / `%ifos` conditional blocks
//! are parsed structurally. Preamble lines, section headers and section
//! bodies emit a "deferred to stage 2/3" diagnostic and are skipped.

pub mod changelog;
pub mod cond;
pub mod deps;
pub mod files;
pub mod input;
pub mod macros;
pub mod preamble;
pub mod scriptlet;
pub mod section;
pub mod state;
pub mod text;
pub mod util;

mod entry;

pub use entry::{parse_str, parse_str_with_spans};
pub use input::{Input, span_at, span_between};
pub use state::{ParserConfig, ParserState};
