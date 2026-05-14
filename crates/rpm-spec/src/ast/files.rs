//! Body of the `%files` section: per-file directives and paths.

use super::cond::Conditional;
use super::macros::Comment;
use super::text::Text;

/// One element of a `%files` body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FilesContent<T = ()> {
    Entry(FileEntry<T>),
    Conditional(Conditional<T, FilesContent<T>>),
    Comment(Comment<T>),
    Blank,
}

/// One logical row inside `%files`: zero or more directives plus an optional
/// path. `%defattr(...)` on its own line yields `path = None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileEntry<T = ()> {
    pub directives: Vec<FileDirective>,
    pub path:       Option<FilePath>,
    pub data:       T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileDirective {
    /// `%defattr(fmode, user, group [, dmode])` — set defaults for the
    /// remainder of the section.
    Defattr {
        fmode: AttrField,
        user:  AttrField,
        group: AttrField,
        dmode: Option<AttrField>,
    },
    /// `%attr(mode, user, group) path` — per-file override.
    Attr {
        mode:  AttrField,
        user:  AttrField,
        group: AttrField,
    },
    /// `%dir path` — own a directory but not its contents.
    Dir,
    /// `%doc path…` — install under `%{_docdir}` when the path is relative.
    Doc,
    /// `%license path…` — install under `%{_defaultlicensedir}` (rpm ≥ 4.11).
    License,
    /// `%config[(noreplace|missingok|…)] path`.
    Config(Vec<ConfigFlag>),
    /// `%ghost path` — own the path but do not package it.
    Ghost,
    /// `%verify([not] checks…) path`.
    Verify { negate: bool, checks: Vec<VerifyCheck> },
    /// `%lang(ru) path` — locale-specific files.
    Lang(Text),
    /// `%caps(cap_…=ep) path` — Linux capabilities.
    Caps(Text),
    /// `%artifact path` (rpm ≥ 4.14.1).
    Artifact,
    /// `%missingok path` (rpm ≥ 4.14).
    MissingOk,
}

/// A single field inside `%defattr(...)` or `%attr(...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AttrField {
    /// `-` — leave the value untouched / default.
    Default,
    /// Numeric octal mode such as `0644` or `0755`.
    Numeric(u32),
    /// User or group name (may contain macros).
    Name(Text),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ConfigFlag {
    NoReplace,
    MissingOk,
}

/// File verification check (`rpm --verify`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum VerifyCheck {
    Md5,
    FileDigest,
    Size,
    Link,
    User,
    Group,
    Mtime,
    Mode,
    Rdev,
    Caps,
}

/// File path, potentially containing macros and shell globs. The parser
/// keeps the path in a single [`Text`] without further decomposition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FilePath {
    pub path: Text,
}
