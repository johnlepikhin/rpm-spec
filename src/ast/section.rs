//! Top-level sections (`%description`, `%prep`, `%files`, `%changelog`, …).
//!
//! Build-script sections (`%prep`, `%build`, `%install`, `%check`, …) and
//! scriptlet/trigger bodies are stored as [`ShellBody`] — sequences of text
//! lines that may contain macro references but are *not* parsed as bash.

#![allow(missing_docs)]

use super::changelog::ChangelogEntry;
use super::files::FilesContent;
use super::preamble::PreambleContent;
use super::scriptlet::{FileTrigger, Scriptlet, Trigger};
use super::text::Text;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Section<T = ()> {
    /// `%description [-n] [sub]` — free-form text body.
    Description {
        subpkg: Option<SubpkgRef>,
        body:   TextBody,
        data:   T,
    },
    /// `%package [-n] sub` — declares a subpackage with its own preamble.
    Package {
        name_arg: PackageName,
        content:  Vec<PreambleContent<T>>,
        data:     T,
    },
    /// `%prep` / `%conf` / `%build` / `%install` / `%check` / `%clean` /
    /// `%generate_buildrequires` — shell bodies.
    BuildScript {
        kind: BuildScriptKind,
        body: ShellBody,
        data: T,
    },
    /// `%files [-n sub] [-f filelist]` — a list of paths plus directives.
    Files {
        subpkg:     Option<SubpkgRef>,
        /// `-f filelist` — repeatable. Each entry is a path (often a macro).
        file_lists: Vec<Text>,
        content:    Vec<FilesContent<T>>,
        data:       T,
    },
    Scriptlet(Scriptlet<T>),
    Trigger(Trigger<T>),
    FileTrigger(FileTrigger<T>),
    /// `%verify [-n sub]` — shell body executed by `rpm --verify`.
    Verify {
        subpkg: Option<SubpkgRef>,
        body:   ShellBody,
        data:   T,
    },
    /// `%changelog` — the single global changelog block.
    Changelog {
        entries: Vec<ChangelogEntry<T>>,
        data:    T,
    },
    /// `%sourcelist` — alternative to numbered `SourceN:` tags.
    SourceList { entries: Vec<Text>, data: T },
    /// `%patchlist` — alternative to numbered `PatchN:` tags.
    PatchList { entries: Vec<Text>, data: T },
    /// `%sepolicy [-n sub]` — SELinux module section (RH family).
    Sepolicy {
        subpkg: Option<SubpkgRef>,
        body:   ShellBody,
        data:   T,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BuildScriptKind {
    Prep,
    /// `%conf` — rpm ≥ 4.18.
    Conf,
    Build,
    Install,
    Check,
    /// `%clean` — deprecated; rpm cleans the buildroot itself.
    Clean,
    /// `%generate_buildrequires` — rpm ≥ 4.15.
    GenerateBuildRequires,
}

/// Argument of `%package`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PackageName {
    /// `%package foo` — appended to the main name: `<main>-foo`.
    Relative(Text),
    /// `%package -n foo` — absolute subpackage name `foo`.
    Absolute(Text),
}

/// `-n SUB` or bare `SUB` modifier on a section header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SubpkgRef {
    /// Bare suffix form: `<main>-SUB`.
    Relative(Text),
    /// `-n` form: absolute subpackage name.
    Absolute(Text),
}

/// Free-form text body (e.g. inside `%description`). Each entry is one line;
/// an empty [`Text`] denotes a blank line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct TextBody {
    pub lines: Vec<Text>,
}

/// Shell body (e.g. inside `%prep`, `%build`, scriptlets). This crate does
/// *not* parse the bash grammar, but it does distinguish literal text from
/// embedded macro references so they can be expanded or printed cleanly.
///
/// Conditional blocks (`%if/%endif`) appearing inside a shell body are kept
/// as text lines, not as structural [`super::cond::Conditional`] nodes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct ShellBody {
    pub lines: Vec<Text>,
}
