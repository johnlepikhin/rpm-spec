//! Macro definitions, build conditions, `%include` directives, and comments.
//!
//! Macro *references* (use sites) live in [`super::text::MacroRef`]; this
//! module covers everything that introduces or annotates source without
//! producing a value at a particular position.

#![allow(missing_docs)]

use super::text::Text;

/// `%define` / `%global` / `%undefine` at the top level of a spec.
///
/// `name`, `opts`, and `body` are preserved verbatim so that a static analyzer
/// can pair them with a distribution registry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct MacroDef<T = ()> {
    pub kind:     MacroDefKind,
    /// Verbatim macro name (no leading `%`).
    pub name:     String,
    /// Raw parametric option string, e.g. `Some("(f:b)")`. Structural parsing
    /// of these is deferred — callers may parse them when needed.
    pub opts:     Option<String>,
    pub body:     Text,
    /// `-e` flag — force eager expansion in `%define`.
    pub eager:    bool,
    /// `-g` flag — force global scope.
    pub global:   bool,
    /// `<l>` modifier — treat body as a literal (no expansion on definition).
    pub literal:  bool,
    /// `<o>` modifier — one-shot caching (rpm ≥ 4.16).
    pub one_shot: bool,
    pub data:     T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MacroDefKind {
    /// `%define` — lazy expansion, scoped inside parametric macros.
    Define,
    /// `%global` — body is expanded immediately, always at global scope.
    Global,
    /// `%undefine` — pop one level of the named macro's definition stack.
    Undefine,
}

/// `%bcond` / `%bcond_with` / `%bcond_without` — build-time toggles.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct BuildCondition<T = ()> {
    pub style:   BuildCondStyle,
    /// Verbatim feature name (without the `bcond_*` prefix).
    pub name:    String,
    /// Default value expression for `%bcond` (rpm ≥ 4.17.1).
    /// `None` for the legacy `%bcond_with` / `%bcond_without` forms.
    pub default: Option<Text>,
    pub data:    T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BuildCondStyle {
    /// `%bcond name DEFAULT` — rpm ≥ 4.17.1.
    Bcond,
    /// `%bcond_with name` — default off, enabled by `--with name`.
    BcondWith,
    /// `%bcond_without name` — default on, disabled by `--without name`.
    BcondWithout,
}

/// `%include path` directive — kept verbatim, never expanded by this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct IncludeDirective<T = ()> {
    pub path: Text,
    pub data: T,
}

/// A comment line.
///
/// `text` is stored as [`Text`] rather than `String` because RPM expands
/// macros inside `#`-style comments (see [`CommentStyle::Hash`]). Keeping
/// macros as AST nodes lets validators flag side-effects that would
/// otherwise be invisible to a casual reader.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Comment<T = ()> {
    pub style: CommentStyle,
    /// Body of the comment without the leading `#` / `%dnl` and without the
    /// single optional space that customarily follows them.
    pub text:  Text,
    pub data:  T,
}

/// How a comment was introduced.
///
/// `Hash` and `Dnl` are *not* interchangeable: RPM expands macros inside `#`
/// comments before discarding them (a known footgun). `%dnl` is the only
/// truly inert comment form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum CommentStyle {
    /// `# comment` — beware: macros inside ARE expanded by RPM at parse time.
    Hash,
    /// `%dnl comment` — fully suppressed; safe place for raw text.
    Dnl,
}
