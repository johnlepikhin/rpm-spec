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
    pub expr: CondExpr,
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum CondExpr {
    /// `%if EXPR` / `%elif EXPR` — expression in the RPM expression language,
    /// kept as raw text including any macros. This crate does not parse the
    /// expression grammar; a future static analyzer may.
    Raw(Text),
    /// `%ifarch x86_64 aarch64` / `%ifarch %{ix86}` —
    /// whitespace-separated arch identifiers, possibly containing macros.
    ArchList(Vec<Text>),
}
