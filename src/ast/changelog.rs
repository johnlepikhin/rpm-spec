//! `%changelog` section entries.
//!
//! Entries appear in reverse chronological order in the source. This crate
//! preserves them in source order; consumers may sort if needed.
//!
//! The openSUSE-style external `.changes` file is out of scope.

#![allow(missing_docs)]

use super::text::Text;

/// One `%changelog` entry.
///
/// `author`, `email`, and `body` use [`Text`] (rather than `String`) because
/// real-world spec files commonly reference macros in these positions
/// (`%{packager}`, `%{name}`, …) and the AST preserves them verbatim for
/// downstream validators.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct ChangelogEntry<T = ()> {
    pub date: ChangelogDate,
    pub author: Text,
    /// Email captured from `< … >` in the header, if present.
    pub email: Option<Text>,
    /// Trailing `- 1.2-3` (may contain macros). `None` when absent.
    pub version: Option<Text>,
    /// Body lines (everything between this header and the next).
    /// Leading `-` markers are kept as-is; the parser does not trim them.
    pub body: Vec<Text>,
    pub data: T,
}

/// A date as it appears in a `%changelog` header.
///
/// The type is *permissive*: implausible values (e.g. `day=99`,
/// `year=1500`) are accepted by the parser and stored verbatim; the
/// parser only emits a
/// [`crate::parse_result::codes::W_IMPLAUSIBLE_CHANGELOG_DATE`]
/// diagnostic. Consumers that need a calendrically valid date should
/// re-check the fields when reading the AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct ChangelogDate {
    pub weekday: Weekday,
    pub month: Month,
    pub day: u8,
    pub year: u16,
}

/// Three-letter day-of-week tokens as they appear in `%changelog` headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// Three-letter month tokens as they appear in `%changelog` headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Month {
    Jan,
    Feb,
    Mar,
    Apr,
    May,
    Jun,
    Jul,
    Aug,
    Sep,
    Oct,
    Nov,
    Dec,
}
