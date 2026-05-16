//! Dependency expressions used in `Requires:`, `BuildRequires:`, `Provides:`,
//! `Conflicts:`, `Obsoletes:`, trigger conditions, and friends.
//!
//! Both classic atoms (`name (op evr)?`) and RPM 4.13+ rich/boolean
//! dependencies (`(foo and bar)`, `(foo if bar else baz)`) are represented.

#![allow(missing_docs)]

use super::text::Text;

/// A single dependency clause.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DepExpr {
    Atom(DepAtom),
    Rich(Box<BoolDep>),
}

/// A classic dependency atom.
///
/// Examples:
/// - `glibc`
/// - `perl(File::Basename)`
/// - `perl-DBI(x86-64) >= 9:1.643-1`
/// - `/usr/bin/awk`
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct DepAtom {
    /// Dependency name. Can be a package name, a `name(provider)` virtual,
    /// or an absolute file path. May contain macros.
    pub name: Text,
    /// The optional architecture qualifier in parentheses, e.g.
    /// `name(x86-64)` → `Some(Text::from("x86-64"))`.
    pub arch: Option<Text>,
    pub constraint: Option<DepConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct DepConstraint {
    pub op: VerOp,
    pub evr: EVR,
}

/// Version comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum VerOp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
    Ne,
}

/// Epoch–Version–Release triple. Epoch and Release are optional.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct EVR {
    pub epoch: Option<u32>,
    pub version: Text,
    pub release: Option<Text>,
}

/// Boolean / rich dependency tree (RPM ≥ 4.13).
///
/// Variants with one or more child expressions box them (`Box<DepExpr>`) to
/// keep the enum's footprint small — without boxing, `If` and friends would
/// inflate every other variant to several hundred bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoolDep {
    And(Vec<DepExpr>),
    Or(Vec<DepExpr>),
    /// `(then if cond)` or `(then if cond else otherwise)`.
    If {
        cond: Box<DepExpr>,
        then: Box<DepExpr>,
        otherwise: Option<Box<DepExpr>>,
    },
    /// `(then unless cond)` or `(then unless cond else otherwise)`.
    Unless {
        cond: Box<DepExpr>,
        then: Box<DepExpr>,
        otherwise: Option<Box<DepExpr>>,
    },
    With(Vec<DepExpr>),
    Without {
        left: Box<DepExpr>,
        right: Box<DepExpr>,
    },
}
