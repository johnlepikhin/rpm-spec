//! Text fragments and macro references — the building blocks for every
//! value-bearing position in the AST.
//!
//! # Verbatim names invariant
//!
//! [`MacroRef::name`] and [`BuiltinMacro::Other`] preserve the *exact*
//! identifier as written in the source. The parser never normalizes case or
//! aliases. Static analyzers that pair the AST with a distribution-specific
//! macro registry rely on this property.

/// A piece of text that mixes literal characters with macro expansions.
///
/// `Text` is the universal carrier for every value that can hold macros:
/// preamble values, file paths, EVR fields, dependency atom names, shell
/// body lines, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Text {
    pub segments: Vec<TextSegment>,
}

impl Text {
    pub const fn new() -> Self {
        Self { segments: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
            || self.segments.iter().all(|s| match s {
                TextSegment::Literal(s) => s.is_empty(),
                TextSegment::Macro(_) => false,
            })
    }

    /// If the text contains exactly one literal segment, returns its content.
    /// Returns `None` if there are macros or multiple segments.
    pub fn literal_str(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [] => Some(""),
            [TextSegment::Literal(s)] => Some(s),
            _ => None,
        }
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self { segments: vec![TextSegment::Literal(s.to_owned())] }
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self { segments: vec![TextSegment::Literal(s)] }
    }
}

impl From<MacroRef> for Text {
    fn from(m: MacroRef) -> Self {
        Self { segments: vec![TextSegment::Macro(m)] }
    }
}

/// A single segment inside [`Text`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TextSegment {
    /// Verbatim characters. `%%` in the source is decoded to a single `%`
    /// in the literal, since `%%` carries no syntactic information beyond
    /// "escaped percent".
    Literal(String),
    Macro(MacroRef),
}

/// A reference (use site) to a macro.
///
/// Macro *definitions* are represented separately by [`super::macros::MacroDef`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MacroRef {
    pub kind:        MacroKind,
    /// Verbatim macro name as written, without the leading `%` and without
    /// `?`/`!?` prefixes. For positional and flag references the name keeps
    /// its sigil (`"1"`, `"*"`, `"**"`, `"#"`, `"-f"`, `"-f*"`).
    pub name:        String,
    /// Arguments passed to a parametric macro (or the raw body for builtins
    /// like `%{shrink:...}`, where there is exactly one element).
    pub args:        Vec<Text>,
    pub conditional: ConditionalMacro,
    /// `%{?foo:VALUE}` or `%{!?foo:VALUE}` — the body after `:` when
    /// [`MacroRef::conditional`] is not [`ConditionalMacro::None`].
    pub with_value:  Option<Text>,
}

impl MacroRef {
    /// `%1`, `%2`, ... → `Some(N)`.
    pub fn positional_index(&self) -> Option<u32> {
        self.name.parse::<u32>().ok()
    }

    /// `%*` — all positional arguments (without option flags).
    pub fn is_all_positional(&self) -> bool {
        self.name == "*"
    }

    /// `%**` — all arguments including option flags.
    pub fn is_all_args(&self) -> bool {
        self.name == "**"
    }

    /// `%#` — argument count.
    pub fn is_arg_count(&self) -> bool {
        self.name == "#"
    }

    /// `%{-f}` → `Some(("f", false))`; `%{-f*}` → `Some(("f", true))`.
    /// The second tuple element is `true` when the flag's *value* is
    /// requested (`-f*`), `false` when only its presence is tested (`-f`).
    pub fn flag_ref(&self) -> Option<(&str, bool)> {
        let rest = self.name.strip_prefix('-')?;
        if let Some(name) = rest.strip_suffix('*') {
            if name.is_empty() {
                None
            } else {
                Some((name, true))
            }
        } else if rest.is_empty() {
            None
        } else {
            Some((rest, false))
        }
    }
}

/// Surface form of a macro reference.
///
/// The variants distinguish how a use site is written in source — not what
/// the macro *does*. The semantics are determined by the macro's definition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Builtin macro function: `%{shrink:...}`, `%{quote:...}`, `%{gsub:...}`,
    /// etc. See [`BuiltinMacro`].
    Builtin(BuiltinMacro),
}

/// Conditional prefix on a macro reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
/// versions; parsers should populate it with the verbatim name.
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
    /// Verbatim name for unknown builtins.
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_classification() {
        let m = MacroRef {
            kind: MacroKind::Plain,
            name: "1".into(),
            args: vec![],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(m.positional_index(), Some(1));
        assert!(!m.is_all_positional());
        assert!(!m.is_all_args());
    }

    #[test]
    fn flag_classification() {
        let f = MacroRef {
            kind: MacroKind::Braced,
            name: "-f".into(),
            args: vec![],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(f.flag_ref(), Some(("f", false)));

        let fstar = MacroRef {
            kind: MacroKind::Braced,
            name: "-f*".into(),
            args: vec![],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(fstar.flag_ref(), Some(("f", true)));
    }

    #[test]
    fn text_from_str() {
        let t = Text::from("hello");
        assert_eq!(t.literal_str(), Some("hello"));
        assert!(!t.is_empty());
    }
}
