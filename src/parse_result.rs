//! Result type and diagnostics returned by the parser.
//!
//! The full table of stable diagnostic identifiers lives in [`codes`].

#![allow(missing_docs)]

use crate::ast::{SpecFile, Span};

/// Stable diagnostic identifiers issued by the parser.
///
/// Errors use the `rpmspec/E####` namespace; warnings use
/// `rpmspec/W####`. Codes are stable across patch releases — adding new
/// ones is allowed, renumbering or repurposing is not.
///
/// Each constant is also documented with the message family it
/// represents and a brief note on when the parser raises it.
pub mod codes {
    // ---------- Errors --------------------------------------------------

    /// Parser failed to consume any input at the current position.
    /// Returned as a last-resort guard to prevent infinite loops.
    pub const E_NO_PROGRESS: &str = "rpmspec/E0001";

    /// A `%if` / `%ifarch` / `%ifos` block opened but the parser
    /// reached EOF (or a synchronisation point) without seeing the
    /// matching `%endif`.
    pub const E_UNTERMINATED_CONDITIONAL: &str = "rpmspec/E0002";

    /// A rich dependency `(...)` expression mixes different operators
    /// on the same level without explicit nesting parentheses, e.g.
    /// `(a and b or c)`.
    pub const E_RICH_DEP_MIXED_OPS: &str = "rpmspec/E0003";

    /// A rich dependency has unbalanced parentheses.
    pub const E_RICH_DEP_UNBALANCED: &str = "rpmspec/E0004";

    /// A rich dependency operand was empty (typical typo: `(a and )`).
    pub const E_RICH_DEP_EMPTY_OPERAND: &str = "rpmspec/E0005";

    /// A classic dependency atom has no name.
    pub const E_DEP_ATOM_NO_NAME: &str = "rpmspec/E0006";

    /// `%package` was declared without a subpackage name argument.
    pub const E_PACKAGE_NEEDS_NAME: &str = "rpmspec/E0007";

    /// `else` keyword used inside a rich dependency without a matching
    /// `if` or `unless`.
    pub const E_ELSE_WITHOUT_IF: &str = "rpmspec/E0008";

    /// An extra operator appeared inside an `if … [else …]` /
    /// `unless … [else …]` rich dependency.
    pub const E_UNEXPECTED_OP_IF_UNLESS: &str = "rpmspec/E0009";

    /// The `without` rich-dependency operator was given a number of
    /// operands different from 2.
    pub const E_RICH_DEP_WITHOUT_ARITY: &str = "rpmspec/E0010";

    // ---------- Warnings ------------------------------------------------

    /// A `%` character appeared in text without forming a valid macro
    /// reference. The character is preserved as a literal, but in real
    /// spec sources it should be written as `%%`.
    pub const W_STRAY_PERCENT: &str = "rpmspec/W0001";

    /// A line at the top level did not match any recognised
    /// construction (preamble, section header, macro statement, etc.).
    pub const W_LINE_NOT_RECOGNIZED: &str = "rpmspec/W0002";

    /// A builtin macro reference was used without its required `:body`
    /// suffix, e.g. `%{shrink}` instead of `%{shrink:…}`.
    pub const W_BUILTIN_MISSING_BODY: &str = "rpmspec/W0003";

    /// A `%(…)`, `%[…]`, or `%{…}` macro reference was not properly
    /// closed before end of input or a terminator.
    pub const W_UNTERMINATED_MACRO: &str = "rpmspec/W0004";

    /// A conditional block contained more than one `%else` clause.
    /// Only the last one is honoured.
    pub const W_MULTIPLE_ELSE: &str = "rpmspec/W0005";

    /// `%elif` appeared after `%else`. It is treated as part of the
    /// `%else` body rather than starting a new branch.
    pub const W_ELIF_AFTER_ELSE: &str = "rpmspec/W0006";

    /// A dep slice in a multi-dep value was empty (typical cause: a
    /// trailing or doubled `,`).
    pub const W_EMPTY_DEP: &str = "rpmspec/W0007";

    /// A scriptlet/trigger header contained an unknown token that did
    /// not match any flag (`-n`/`-p`/`-e`/`-q`/`-f`/`-P`) or look like
    /// a bare subpkg name.
    pub const W_UNKNOWN_SCRIPTLET_TOKEN: &str = "rpmspec/W0008";

    /// A `--` separator appeared in a scriptlet header that does not
    /// expect one (scriptlets, unlike triggers, do not take `--`).
    pub const W_SCRIPTLET_DASHES_INVALID: &str = "rpmspec/W0009";

    /// `%files -f` was used without a following filelist path.
    pub const W_EXPECTED_FILELIST: &str = "rpmspec/W0010";

    /// A scriptlet/trigger `-p` flag was used without a following
    /// interpreter token.
    pub const W_EXPECTED_INTERP: &str = "rpmspec/W0011";

    /// A scriptlet/trigger `-P` flag was used without a valid numeric
    /// priority.
    pub const W_EXPECTED_PRIORITY: &str = "rpmspec/W0012";

    /// A header used `-n` without a following name token.
    pub const W_EXPECTED_NAME_AFTER_N: &str = "rpmspec/W0013";

    /// A scriptlet `-f` flag was used without a following file path.
    pub const W_EXPECTED_FILE_AFTER_F: &str = "rpmspec/W0014";

    /// A trigger header lacked the `--` separator that introduces the
    /// list of conditions.
    pub const W_TRIGGER_MISSING_DASHES: &str = "rpmspec/W0015";

    /// A file-trigger header lacked the `--` separator that introduces
    /// the list of path prefixes.
    pub const W_FILE_TRIGGER_MISSING_DASHES: &str = "rpmspec/W0016";

    /// A boolean-shaped tag (`AutoReq`, `AutoProv`, `AutoReqProv`) had
    /// a value that wasn't one of `0`/`1`/`yes`/`no`/`true`/`false`
    /// (case-insensitive).
    pub const W_INVALID_BOOL: &str = "rpmspec/W0017";

    /// A numeric-shaped tag (currently `Epoch`) had a non-integer
    /// value. Falls back to `TagValue::Text` with a warning.
    pub const W_INVALID_NUMBER: &str = "rpmspec/W0018";

    /// A line inside a `%files` body did not match a directive, a
    /// path, a comment, or a nested `%if` block.
    pub const W_LINE_NOT_RECOGNIZED_IN_FILES: &str = "rpmspec/W0019";

    /// A line inside a `%package` body did not match a preamble item,
    /// a comment, a blank line, or a nested `%if` block.
    pub const W_LINE_NOT_RECOGNIZED_IN_PACKAGE: &str = "rpmspec/W0020";

    /// A macro reference appeared with an empty or otherwise invalid
    /// name (e.g. `%{}` or `%{ }`).
    pub const W_MACRO_EMPTY_NAME: &str = "rpmspec/W0021";

    /// A non-blank line inside a `%changelog` section preceded the
    /// first `*`-headed entry.
    pub const W_UNEXPECTED_LINE_IN_CHANGELOG: &str = "rpmspec/W0022";

    /// A `%changelog` entry header (`* Weekday Month Day Year …`)
    /// could not be parsed.
    ///
    /// **Scope note:** this code fires only when the header is
    /// structurally unparseable. A header that parses but carries an
    /// implausible day-of-month or year is reported with
    /// [`W_IMPLAUSIBLE_CHANGELOG_DATE`] instead.
    pub const W_MALFORMED_CHANGELOG_HEADER: &str = "rpmspec/W0023";

    /// A section name was recognised but no structural body parser
    /// exists for it yet — its body is swallowed and replaced with a
    /// placeholder comment. Currently unused (every section landed in
    /// Stage 3); retained for forward compatibility.
    pub const W_DEFERRED_SECTION: &str = "rpmspec/W0024";

    /// A `%changelog` entry header parsed successfully but contains an
    /// implausible date (day outside 1..=31 or year outside a
    /// reasonable range). Distinct from
    /// [`W_MALFORMED_CHANGELOG_HEADER`], which fires when the header
    /// could not be parsed at all.
    pub const W_IMPLAUSIBLE_CHANGELOG_DATE: &str = "rpmspec/W0025";
}

/// Outcome of parsing a `.spec` source.
///
/// The parser always returns a [`SpecFile`] (even if partial) along with a
/// list of recoverable issues. Fatal errors that prevent producing any AST
/// are signalled by [`crate::error::ParseError`] from the parser entry
/// points.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub struct ParseResult<T = ()> {
    pub spec:        SpecFile<T>,
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> ParseResult<T> {
    pub fn new(spec: SpecFile<T>) -> Self
    where
        Vec<Diagnostic>: Default,
    {
        Self { spec, diagnostics: Vec::new() }
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}

/// A recoverable issue surfaced by the parser (or, in the future, by a
/// validator pass).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable diagnostic identifier intended for suppression and filtering
    /// (e.g. `"rpmspec/E0001"`, `"rpmspec/W042"`). `None` for ad-hoc
    /// diagnostics that have not been categorized yet.
    pub code:     Option<String>,
    pub span:     Option<Span>,
    pub message:  String,
    /// Free-form additional context lines shown alongside the main message.
    pub notes:    Vec<String>,
}

impl Diagnostic {
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code: None,
            span: None,
            message: message.into(),
            notes: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code: None,
            span: None,
            message: message.into(),
            notes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Severity {
    Warning,
    Error,
}
