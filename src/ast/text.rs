//! Text fragments and macro references — the building blocks for every
//! value-bearing position in the AST.
//!
//! # Verbatim names invariant
//!
//! [`MacroRef::name`] and [`BuiltinMacro::Other`] preserve the *exact*
//! identifier as written in the source. The parser never normalizes case or
//! aliases. Static analyzers that pair the AST with a distribution-specific
//! macro registry rely on this property.
//!
//! # Round-trip and `%%`
//!
//! `%%` in source decodes to a single `%` inside [`TextSegment::Literal`].
//! The pretty-printer is responsible for re-escaping a stray `%` to `%%`
//! when emitting text in a context where it would otherwise be interpreted
//! as a macro start.

#![allow(missing_docs)]

/// Sigil used by `MacroRef::name` to mark `%*` — all positional args.
pub const SIGIL_ALL_POSITIONAL: &str = "*";

/// Sigil used by `MacroRef::name` to mark `%**` — all args including flags.
pub const SIGIL_ALL_ARGS: &str = "**";

/// Sigil used by `MacroRef::name` to mark `%#` — argument count.
pub const SIGIL_ARG_COUNT: &str = "#";

/// A piece of text that mixes literal characters with macro expansions.
///
/// `Text` is the universal carrier for every value that can hold macros:
/// preamble values, file paths, EVR fields, dependency atom names, shell
/// body lines, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Text {
    pub segments: Vec<TextSegment>,
}

impl Text {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Returns `true` if the text has no segments or all segments are empty
    /// literals.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments
            .iter()
            .all(|s| matches!(s, TextSegment::Literal(literal) if literal.is_empty()))
    }

    /// If the text consists of a single literal segment (or is empty),
    /// returns that literal. Returns `None` when any macro is present.
    #[must_use]
    pub fn literal_str(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [] => Some(""),
            [TextSegment::Literal(s)] => Some(s),
            _ => None,
        }
    }

    /// Iterate over the segments by reference.
    pub fn iter(&self) -> std::slice::Iter<'_, TextSegment> {
        self.segments.iter()
    }
}

impl<'a> IntoIterator for &'a Text {
    type Item = &'a TextSegment;
    type IntoIter = std::slice::Iter<'a, TextSegment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.iter()
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self {
            segments: vec![TextSegment::Literal(s.to_owned())],
        }
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self {
            segments: vec![TextSegment::Literal(s)],
        }
    }
}

impl From<MacroRef> for Text {
    fn from(m: MacroRef) -> Self {
        Self {
            segments: vec![TextSegment::Macro(Box::new(m))],
        }
    }
}

/// A single segment inside [`Text`].
///
/// The [`TextSegment::Macro`] variant is boxed because [`MacroRef`] is
/// substantially larger than a [`String`]; without boxing every literal
/// segment would pay for the macro-shaped padding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TextSegment {
    /// Verbatim characters. `%%` in source decodes to a single `%` here.
    Literal(String),
    Macro(Box<MacroRef>),
}

impl TextSegment {
    /// Convenience constructor that boxes the [`MacroRef`].
    pub fn macro_ref(m: MacroRef) -> Self {
        Self::Macro(Box::new(m))
    }
}

/// A reference (use site) to a macro.
///
/// Macro *definitions* are represented separately by
/// [`crate::ast::MacroDef`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct MacroRef {
    pub kind: MacroKind,
    /// Verbatim macro name as written, without the leading `%` and without
    /// `?` / `!?` prefixes. For positional and flag references the name
    /// keeps its sigil (`"1"`, `"*"`, `"**"`, `"#"`, `"-f"`, `"-f*"`).
    pub name: String,
    /// Arguments passed to a parametric macro (or the raw body for builtins
    /// like `%{shrink:...}`, where there is exactly one element).
    pub args: Vec<Text>,
    pub conditional: ConditionalMacro,
    /// `%{?foo:VALUE}` / `%{!?foo:VALUE}` — the body after `:` when
    /// [`MacroRef::conditional`] is not [`ConditionalMacro::None`].
    pub with_value: Option<Text>,
}

impl MacroRef {
    /// `%1`, `%2`, ... → `Some(N)`.
    ///
    /// Leading-zero forms such as `"01"` are rejected (the parser would not
    /// produce them; this method should not silently normalize them either).
    #[must_use]
    pub fn positional_index(&self) -> Option<u32> {
        if self.name.len() > 1 && self.name.starts_with('0') {
            return None;
        }
        self.name.parse::<u32>().ok()
    }

    /// `%*` — all positional arguments (without option flags).
    #[must_use]
    pub fn is_all_positional(&self) -> bool {
        self.name == SIGIL_ALL_POSITIONAL
    }

    /// `%**` — all arguments including option flags.
    #[must_use]
    pub fn is_all_args(&self) -> bool {
        self.name == SIGIL_ALL_ARGS
    }

    /// `%#` — argument count.
    #[must_use]
    pub fn is_arg_count(&self) -> bool {
        self.name == SIGIL_ARG_COUNT
    }

    /// `%{-f}` → `Some(("f", false))`; `%{-f*}` → `Some(("f", true))`.
    ///
    /// The boolean is `true` when the flag's *value* is requested
    /// (`-f*`), `false` when only its presence is tested (`-f`). Returns
    /// `None` for names that do not begin with `-`, for the bare `-`, and
    /// for `-*` (empty flag name).
    #[must_use]
    pub fn flag_ref(&self) -> Option<(&str, bool)> {
        let rest = self.name.strip_prefix('-')?;
        match rest.strip_suffix('*') {
            Some("") | None if rest.is_empty() => None,
            Some(name) if !name.is_empty() => Some((name, true)),
            Some(_) => None,
            None => Some((rest, false)),
        }
    }
}

/// Surface form of a macro reference.
///
/// The variants distinguish how a use site is written in source — not what
/// the macro *does*. The semantics are determined by the macro's definition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MacroKind {
    /// `%foo`
    Plain,
    /// `%{foo}`
    Braced,
    /// `%{foo arg1 arg2}` — a parametric macro applied with arguments.
    Parametric,
    /// `%(shell command)` — shell expansion.
    Shell,
    /// `%[expr]` or `%{expr:...}` — RPM expression language.
    Expr,
    /// `%{lua:...}` — embedded Lua.
    Lua,
    /// Builtin macro function: `%{shrink:...}`, `%{quote:...}`,
    /// `%{gsub:...}`, etc. See [`BuiltinMacro`].
    Builtin(BuiltinMacro),
}

/// Conditional prefix on a macro reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConditionalMacro {
    /// `%{foo}` — no prefix.
    None,
    /// `%{?foo}` — expand only if `foo` is defined.
    IfDefined,
    /// `%{!?foo}` — expand only if `foo` is *not* defined.
    IfNotDefined,
}

/// Builtin macro functions recognized by RPM.
///
/// `Other` is a catch-all for builtins that may be added by future RPM
/// versions; parsers should populate it with the verbatim name. The payload
/// is stored as `Box<str>` rather than `String` because the name is
/// immutable after construction and does not need a separate capacity field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BuiltinMacro {
    Expand,
    Expr,
    Shrink,
    Quote,
    Gsub,
    Sub,
    Len,
    Upper,
    Lower,
    Reverse,
    Basename,
    Dirname,
    Suffix,
    Exists,
    Load,
    Echo,
    Warn,
    Error,
    Dnl,
    Trace,
    Dump,
    /// `%{with NAME}` — RPM build-time conditional query. Returns
    /// `"1"` at build time iff the bcond `NAME` is enabled (declared
    /// by `%bcond_with NAME` + `--with NAME`, or by
    /// `%bcond_without NAME` without `--without NAME`).
    ///
    /// The feature name lives in [`MacroRef::args`]`[0]` as a
    /// single-segment [`Text`] — same shape every other parametric
    /// builtin uses (`%{shrink:body}` stores `body` in `args[0]`).
    /// Unit variant (no payload) so there is exactly one source of
    /// truth for the feature name.
    With,
    /// `%{without NAME}` — inverse of [`Self::With`]. Returns `"1"`
    /// iff the bcond is disabled. Feature name in `args[0]`.
    Without,
    /// Verbatim name for unknown builtins.
    Other(Box<str>),
}

/// Surface kind of a `%{with NAME}` / `%{without NAME}` query.
/// Used by the [`parse_bcond_verbatim`] helper so consumers don't
/// reimplement the substring matching every time they need to
/// detect a bcond query from raw source text (e.g. inside a
/// [`crate::ast::expr::ExprAst::Macro`] verbatim body where the
/// parser does NOT structurally model the inner macro reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BcondForm {
    /// `%{with FEATURE}`.
    With,
    /// `%{without FEATURE}`.
    Without,
}

/// Parse a verbatim macro-reference token like `"%{with bootstrap}"`
/// or `"%{without docs}"` and recover the bcond form plus the
/// feature name. Returns `None` if the token isn't a bcond shape.
///
/// Centralised here (rather than in every analyzer / LSP / formatter
/// that wants to recognise bcond queries) so the parsing rule has a
/// single source of truth. Concretely:
///
/// * Wrapper `%{ ... }` is required; leading `%name` plain forms or
///   `%(...)` shells are rejected.
/// * Exactly one whitespace-separated argument is required after
///   the keyword. `%{with}` (no arg), `%{with foo bar}` (two args)
///   and `%{withholding}` (no space) all return `None`.
/// * Whitespace inside the braces is tolerated (`%{ with foo }`).
/// * Conditional prefixes `%{?with foo}` and `%{!?with foo}` are
///   stripped before matching the keyword — they are valid RPM
///   surface forms for the same bcond query.
///
/// The returned `&str` borrows from the input; the caller copies if
/// it needs an owned name.
#[must_use]
pub fn parse_bcond_verbatim(verbatim: &str) -> Option<(BcondForm, &str)> {
    let inner = verbatim.strip_prefix("%{")?.strip_suffix('}')?;
    // Tolerate `?` / `!?` conditional prefixes — RPM treats
    // `%{?with foo}` as the same bcond query as `%{with foo}` (the
    // `?` triggers "expand only if defined", but `with` is always
    // a builtin so it always expands). Aligning here so analyzer
    // and lint share one detection rule.
    let inner = inner.trim();
    let inner = inner
        .strip_prefix("!?")
        .or_else(|| inner.strip_prefix('?'))
        .unwrap_or(inner)
        .trim_start();
    let mut parts = inner.splitn(2, char::is_whitespace);
    let keyword = parts.next()?;
    let name = parts.next()?.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let form = match keyword {
        "with" => BcondForm::With,
        "without" => BcondForm::Without,
        _ => return None,
    };
    Some((form, name))
}

#[cfg(test)]
mod bcond_parse_tests {
    use super::*;

    #[test]
    fn parses_with_form() {
        assert_eq!(
            parse_bcond_verbatim("%{with bootstrap}"),
            Some((BcondForm::With, "bootstrap"))
        );
    }

    #[test]
    fn parses_without_form() {
        assert_eq!(
            parse_bcond_verbatim("%{without docs}"),
            Some((BcondForm::Without, "docs"))
        );
    }

    #[test]
    fn tolerates_inner_whitespace() {
        assert_eq!(
            parse_bcond_verbatim("%{ with foo }"),
            Some((BcondForm::With, "foo"))
        );
    }

    #[test]
    fn rejects_missing_arg() {
        assert_eq!(parse_bcond_verbatim("%{with}"), None);
    }

    #[test]
    fn rejects_two_args() {
        // RPM treats `%{with foo bar}` as a 2-arg parametric call,
        // not a bcond query.
        assert_eq!(parse_bcond_verbatim("%{with foo bar}"), None);
    }

    #[test]
    fn rejects_prefix_only_match() {
        // `withholding` mustn't be matched as a "with" keyword.
        assert_eq!(parse_bcond_verbatim("%{withholding}"), None);
    }

    #[test]
    fn rejects_plain_form() {
        // No braces — not a bcond verbatim shape.
        assert_eq!(parse_bcond_verbatim("%with foo"), None);
    }

    #[test]
    fn rejects_unrelated_keyword() {
        assert_eq!(parse_bcond_verbatim("%{shrink foo}"), None);
    }

    #[test]
    fn accepts_conditional_question_prefix() {
        // `%{?with foo}` is valid RPM — `?` means "expand if
        // defined", and `with` is always a builtin so it always
        // expands. Treat as the same query.
        assert_eq!(
            parse_bcond_verbatim("%{?with foo}"),
            Some((BcondForm::With, "foo"))
        );
    }

    #[test]
    fn accepts_conditional_bang_question_prefix() {
        assert_eq!(
            parse_bcond_verbatim("%{!?without docs}"),
            Some((BcondForm::Without, "docs"))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macro_named(name: &str) -> MacroRef {
        MacroRef {
            kind: MacroKind::Braced,
            name: name.into(),
            args: Vec::new(),
            conditional: ConditionalMacro::None,
            with_value: None,
        }
    }

    #[test]
    fn positional_index_basic() {
        assert_eq!(macro_named("1").positional_index(), Some(1));
        assert_eq!(macro_named("9").positional_index(), Some(9));
        assert_eq!(macro_named("0").positional_index(), Some(0));
    }

    #[test]
    fn positional_index_rejects_leading_zero_compound() {
        assert_eq!(macro_named("01").positional_index(), None);
        assert_eq!(macro_named("007").positional_index(), None);
    }

    #[test]
    fn positional_index_rejects_non_numeric() {
        assert_eq!(macro_named("foo").positional_index(), None);
        assert_eq!(macro_named("*").positional_index(), None);
        assert_eq!(macro_named("-f").positional_index(), None);
    }

    #[test]
    fn all_positional_args_count_classifiers() {
        assert!(macro_named("*").is_all_positional());
        assert!(!macro_named("1").is_all_positional());
        assert!(!macro_named("**").is_all_positional());

        assert!(macro_named("**").is_all_args());
        assert!(!macro_named("*").is_all_args());

        assert!(macro_named("#").is_arg_count());
        assert!(!macro_named("1").is_arg_count());
    }

    #[test]
    fn flag_ref_named() {
        assert_eq!(macro_named("-f").flag_ref(), Some(("f", false)));
        assert_eq!(macro_named("-foo").flag_ref(), Some(("foo", false)));
        assert_eq!(macro_named("-f*").flag_ref(), Some(("f", true)));
        assert_eq!(macro_named("-foo*").flag_ref(), Some(("foo", true)));
    }

    #[test]
    fn flag_ref_rejects_degenerate() {
        assert_eq!(macro_named("-").flag_ref(), None);
        assert_eq!(macro_named("-*").flag_ref(), None);
        assert_eq!(macro_named("foo").flag_ref(), None);
        assert_eq!(macro_named("*").flag_ref(), None);
    }

    #[test]
    fn text_from_str() {
        let t = Text::from("hello");
        assert_eq!(t.literal_str(), Some("hello"));
        assert!(!t.is_empty());
    }

    #[test]
    fn text_is_empty_with_macro_is_false() {
        let t: Text = macro_named("foo").into();
        assert!(!t.is_empty());
        assert_eq!(t.literal_str(), None);
    }

    #[test]
    fn text_literal_str_empty() {
        let t = Text::new();
        assert_eq!(t.literal_str(), Some(""));
        assert!(t.is_empty());
    }

    #[test]
    fn text_iter_yields_segments() {
        let t = Text::from("abc");
        assert_eq!(t.iter().count(), 1);
        let count: usize = (&t).into_iter().count();
        assert_eq!(count, 1);
    }
}
