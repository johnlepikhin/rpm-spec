//! Scriptlets, triggers, and file triggers.
//!
//! See `man rpm-scriptlets` for runtime semantics.

#![allow(missing_docs)]

use super::deps::DepExpr;
use super::section::{ShellBody, SubpkgRef};
use super::text::Text;

/// RPM's default priority value for `%filetrigger*` declarations when no
/// `-P` is given.
pub const DEFAULT_FILE_TRIGGER_PRIORITY: u32 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Scriptlet<T = ()> {
    pub kind: ScriptletKind,
    pub subpkg: Option<SubpkgRef>,
    /// `-p` — interpreter selection.
    pub interp: Option<Interpreter>,
    /// `-e` — expand macros in the body before execution.
    pub expand_macros: bool,
    /// `-q` — quiet mode.
    pub quiet: bool,
    /// `-f FILE` — body read from `FILE` instead of inline.
    pub from_file: Option<Text>,
    pub body: ShellBody<T>,
    pub data: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ScriptletKind {
    Pre,
    Post,
    Preun,
    Postun,
    Pretrans,
    Posttrans,
    /// `%preuntrans` — rpm ≥ 4.19.
    Preuntrans,
    /// `%postuntrans` — rpm ≥ 4.19.
    Postuntrans,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Interpreter {
    /// `-p /bin/sh`, `-p /usr/bin/python3`, etc.
    Path(Text),
    /// `-p <lua>` — embedded Lua.
    Lua,
}

/// `%triggerprein` / `%triggerin` / `%triggerun` / `%triggerpostun`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Trigger<T = ()> {
    pub kind: TriggerKind,
    pub subpkg: Option<SubpkgRef>,
    pub interp: Option<Interpreter>,
    /// Conditions written after `--` and separated by commas.
    pub conditions: Vec<DepExpr>,
    pub body: ShellBody<T>,
    pub data: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TriggerKind {
    Prein,
    In,
    Un,
    Postun,
}

/// File triggers (rpm ≥ 4.13). `Trans*` variants run once per transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct FileTrigger<T = ()> {
    pub kind: FileTriggerKind,
    pub subpkg: Option<SubpkgRef>,
    pub interp: Option<Interpreter>,
    /// `-P NN` — priority. When `None`, RPM defaults to
    /// [`DEFAULT_FILE_TRIGGER_PRIORITY`]; higher values run earlier.
    pub priority: Option<u32>,
    /// Path prefixes written after `--`.
    pub prefixes: Vec<Text>,
    pub body: ShellBody<T>,
    pub data: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileTriggerKind {
    In,
    Un,
    Postun,
    TransIn,
    TransUn,
    TransPostun,
}
