//! Preamble items: `Tag: value` pairs that introduce a package or subpackage.
//!
//! Tag names are case-insensitive in RPM but the canonical CamelCase form is
//! used in [`Tag`]. Unknown or distribution-specific tags fall into
//! [`Tag::Other`] which preserves the verbatim spelling.

use super::cond::Conditional;
use super::deps::DepExpr;
use super::macros::Comment;
use super::text::Text;

/// Body of `%package` (and the implicit "main" preamble) — a list of
/// preamble items with optional comments, blank lines, and `%if` blocks
/// interleaved.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PreambleContent<T = ()> {
    Item(PreambleItem<T>),
    Conditional(Conditional<T, PreambleContent<T>>),
    Comment(Comment<T>),
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PreambleItem<T = ()> {
    pub tag:        Tag,
    /// Qualifiers in parentheses after the tag name, e.g.
    /// `Requires(post,postun)` → `vec![Post, Postun]`.
    pub qualifiers: Vec<TagQualifier>,
    /// Language qualifier in parentheses, e.g.
    /// `Summary(ru_RU.UTF-8): …` → `Some("ru_RU.UTF-8")`.
    pub lang:       Option<String>,
    pub value:      TagValue,
    pub data:       T,
}

/// Canonical preamble tag.
///
/// `Source(Option<u32>)` distinguishes the bare form `Source:` (`None`) from
/// the numbered form `Source0:` / `Source5:` (`Some(N)`) so the printer can
/// reproduce the original surface form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Tag {
    Name,
    Version,
    Release,
    Summary,
    License,
    URL,
    Group,
    Epoch,
    Icon,

    Source(Option<u32>),
    Patch(Option<u32>),
    NoSource(u32),
    NoPatch(u32),

    Requires,
    BuildRequires,
    Provides,
    Conflicts,
    Obsoletes,
    Recommends,
    Suggests,
    Supplements,
    Enhances,
    BuildConflicts,
    OrderWithRequires,

    BuildArch,
    ExclusiveArch,
    ExcludeArch,
    ExclusiveOS,
    ExcludeOS,

    BuildRoot,
    Distribution,
    Vendor,
    Packager,

    AutoReq,
    AutoProv,
    AutoReqProv,

    Prefix,
    Prefixes,

    BugURL,
    ModularityLabel,
    VCS,

    /// Verbatim name of an unknown / distribution-specific tag.
    Other(String),
}

/// Parenthesized qualifier of a dependency-bearing tag, e.g.
/// `Requires(post,postun)` or `Requires(meta)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TagQualifier {
    Pre,
    Post,
    Preun,
    Postun,
    Pretrans,
    Posttrans,
    Preuntrans,
    Postuntrans,
    Verify,
    Interp,
    /// `meta` (rpm ≥ 4.16).
    Meta,
    /// Verbatim qualifier name when unrecognised.
    Other(String),
}

/// Tag value. The parser chooses the structural variant based on the tag
/// kind; the [`Text`] variant is the universal fallback for free-form values.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TagValue {
    /// Free-form text (URL, Summary, Source path, License expression, …).
    Text(Text),
    /// A dependency expression — for `Requires`, `BuildRequires`, `Provides`,
    /// `Conflicts`, `Obsoletes`, `Recommends`, `Suggests`, `Supplements`,
    /// `Enhances`, `BuildConflicts`, `OrderWithRequires`.
    Dep(DepExpr),
    /// `AutoReq`, `AutoProv`, `AutoReqProv` — accept `0`/`1`/`yes`/`no`.
    Bool(bool),
    /// `Epoch:` — a non-negative integer.
    Number(u32),
    /// Whitespace-separated architecture identifiers, for `BuildArch`,
    /// `ExclusiveArch`, `ExcludeArch`, `ExclusiveOS`, `ExcludeOS`. Each
    /// element is a [`Text`] because identifiers commonly include macros
    /// like `%{ix86}`.
    ArchList(Vec<Text>),
}
