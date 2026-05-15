//! `%if` / `%ifarch` / `%ifos` conditional blocks.
//!
//! The body of each branch is parameterized by `Body` so that the same
//! [`Conditional`] type can wrap any list of items:
//!
//! - top-level: `Conditional<T, SpecItem<T>>`
//! - inside `%package` preamble: `Conditional<T, PreambleContent<T>>`
//! - inside `%files`: `Conditional<T, FilesContent<T>>`
//!
//! Inside shell-style bodies (`%prep`, `%build`, `%install`, scriptlets,
//! triggers) conditional blocks are *not* parsed structurally — they live as
//! ordinary [`super::section::ShellBody`] lines containing macro references.

#![allow(missing_docs)]

use super::expr::ExprAst;
use super::text::Text;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Conditional<T, Body> {
    /// First branch is the `%if` / `%ifarch` / `%ifos` head; further branches
    /// correspond to `%elif*` clauses, in source order.
    pub branches:  Vec<CondBranch<T, Body>>,
    /// Body of the `%else` branch, if any.
    pub otherwise: Option<Vec<Body>>,
    pub data:      T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CondBranch<T, Body> {
    pub kind: CondKind,
    pub expr: CondExpr<T>,
    pub body: Vec<Body>,
    pub data: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum CondKind {
    If,
    IfArch,
    IfNArch,
    IfOs,
    IfNOs,
    Elif,
    ElifArch,
    ElifOs,
}

/// Expression carried by a `%if`/`%elif`/`%ifarch`/`%ifos` branch.
///
/// Generic over `T`: parser fills it with [`crate::ast::Span`] for the
/// span-aware path and `()` otherwise. Only the [`CondExpr::Parsed`]
/// variant actually carries `T` (on every node of its sub-tree).
/// `Raw` and `ArchList` keep their content type unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum CondExpr<T = ()> {
    /// `%if EXPR` / `%elif EXPR` — expression as raw text. Used as a
    /// fallback when the parser couldn't build a structured
    /// [`CondExpr::Parsed`] (malformed grammar, arithmetic that the
    /// AST doesn't model, exotic constructs). Consumers should match
    /// on [`CondExpr::Parsed`] first and fall back to this variant.
    Raw(Text),
    /// Parsed expression with per-node user-data (typically [`crate::ast::Span`]
    /// when produced by the span-aware parser). The parser produces
    /// this variant when the whole expression fits the modelled
    /// grammar (see [`crate::ast::expr`]). When it doesn't, the
    /// parser emits [`CondExpr::Raw`] instead.
    Parsed(Box<ExprAst<T>>),
    /// `%ifarch x86_64 aarch64` / `%ifarch %{ix86}` —
    /// whitespace-separated arch identifiers, possibly containing macros.
    ArchList(Vec<Text>),
}
