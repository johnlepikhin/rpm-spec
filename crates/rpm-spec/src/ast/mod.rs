//! Abstract syntax tree for RPM `.spec` files.
//!
//! The root type is [`SpecFile<T>`]. `T` is a user-data type carried on every
//! "large" node (sections, preamble items, file entries, scriptlets, …) and
//! defaults to `()`. The parser fills `T` with [`Span`] byte ranges when
//! invoked through the span-aware API.
//!
//! # Module map
//!
//! - [`text`]      — [`Text`] / [`TextSegment`] / [`MacroRef`] — the building
//!                   blocks for every value-bearing position in the AST.
//! - [`preamble`]  — `Tag: value` items and the `Tag` enum.
//! - [`section`]   — top-level sections (`%description`, `%prep`, `%files`,
//!                   `%changelog`, …).
//! - [`scriptlet`] — scriptlets and triggers.
//! - [`files`]     — `%files` directives (`%attr`, `%defattr`, `%config`, …).
//! - [`deps`]      — dependency expressions including rich/boolean deps.
//! - [`changelog`] — `%changelog` entries.
//! - [`cond`]      — `%if` / `%ifarch` / `%ifos` blocks (generic over body).
//! - [`macros`]    — `%define` / `%global` / `%bcond` / `%include` / comments.
//! - [`span`]      — [`Span`] byte offsets.

pub mod changelog;
pub mod cond;
pub mod deps;
pub mod files;
pub mod macros;
pub mod preamble;
pub mod scriptlet;
pub mod section;
pub mod span;
pub mod text;

pub use changelog::{ChangelogDate, ChangelogEntry, Month, Weekday};
pub use cond::{CondBranch, CondExpr, CondKind, Conditional};
pub use deps::{BoolDep, DepAtom, DepConstraint, DepExpr, EVR, VerOp};
pub use files::{
    AttrField, ConfigFlag, FileDirective, FileEntry, FilePath, FilesContent, VerifyCheck,
};
pub use macros::{
    BuildCondStyle, BuildCondition, Comment, CommentStyle, IncludeDirective, MacroDef,
    MacroDefKind,
};
pub use preamble::{PreambleContent, PreambleItem, Tag, TagQualifier, TagValue};
pub use scriptlet::{
    FileTrigger, FileTriggerKind, Interpreter, Scriptlet, ScriptletKind, Trigger, TriggerKind,
};
pub use section::{BuildScriptKind, PackageName, Section, ShellBody, SubpkgRef, TextBody};
pub use span::Span;
pub use text::{BuiltinMacro, ConditionalMacro, MacroKind, MacroRef, Text, TextSegment};

/// The root of a parsed `.spec` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpecFile<T = ()> {
    pub items: Vec<SpecItem<T>>,
    pub data:  T,
}

/// A top-level item in a `.spec` file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SpecItem<T = ()> {
    /// A preamble `Tag: value` line (outside any `%package`).
    Preamble(PreambleItem<T>),
    /// A section header and its body.
    Section(Section<T>),
    /// Top-level `%if` / `%ifarch` / `%ifos` block wrapping further items.
    Conditional(Conditional<T, SpecItem<T>>),
    /// `%define` / `%global` / `%undefine`.
    MacroDef(MacroDef<T>),
    /// `%bcond` / `%bcond_with` / `%bcond_without` — distinct from a plain
    /// `MacroDef` because the validator treats build toggles specially.
    BuildCondition(BuildCondition<T>),
    /// `%include`.
    Include(IncludeDirective<T>),
    /// `#` or `%dnl` comment.
    Comment(Comment<T>),
    /// A blank source line, kept so the printer can preserve paragraphing
    /// between top-level items.
    Blank,
}
