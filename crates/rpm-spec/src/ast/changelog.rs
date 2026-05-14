//! `%changelog` section entries.
//!
//! Entries appear in reverse chronological order in the source. This crate
//! preserves them in source order; consumers may sort if needed.
//!
//! The openSUSE-style external `.changes` file is out of scope.

use super::text::Text;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChangelogEntry<T = ()> {
    pub date:    ChangelogDate,
    pub author:  String,
    /// Email captured from `< … >` in the header, if present.
    pub email:   Option<String>,
    /// Trailing `- 1.2-3` (may contain macros). `None` when absent.
    pub version: Option<Text>,
    /// Body lines (everything between this header and the next).
    /// Leading `-` markers are kept as-is; the parser does not trim them.
    pub body:    Vec<String>,
    pub data:    T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChangelogDate {
    pub weekday: Weekday,
    pub month:   Month,
    pub day:     u8,
    pub year:    u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
