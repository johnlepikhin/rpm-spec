//! Shared parser state: configuration plus a recovery-mode diagnostic
//! accumulator.
//!
//! State is passed by reference to every sub-parser. The diagnostics
//! collection is wrapped in `Rc<RefCell<…>>` so sub-parsers can append from
//! deep within nom combinators without threading a `&mut`.

#![allow(missing_docs)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::Span;
use crate::parse_result::{Diagnostic, Severity};

/// Configuration knobs for the parser. Currently a placeholder; future
/// fields will toggle strict vs lenient behaviour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParserConfig {}

impl ParserConfig {
    pub const fn new() -> Self {
        Self {}
    }
}

/// Mutable, cheaply-cloned parser context.
///
/// `ParserState` is passed by shared reference into every sub-parser.
/// Diagnostics live behind `RefCell` so combinator chains can append
/// findings without needing `&mut` propagation.
///
/// # Thread safety
///
/// `ParserState` is neither [`Send`] nor [`Sync`] — both are
/// inherited-not-implemented because the inner `Rc<RefCell<...>>` is
/// itself single-threaded. To parse multiple specs in parallel,
/// create a fresh state per thread:
///
/// ```ignore
/// use rpm_spec::parser::parse_str;
/// let specs: Vec<&str> = vec![/* ... */];
/// // `parse_str` allocates a brand-new ParserState per call, so each
/// // thread gets its own.
/// let results: Vec<_> = specs.iter().map(|s| parse_str(s)).collect();
/// ```
#[derive(Debug, Clone)]
pub struct ParserState {
    pub config:      Rc<ParserConfig>,
    pub diagnostics: Rc<RefCell<Vec<Diagnostic>>>,
}

impl ParserState {
    pub fn new() -> Self {
        Self {
            config:      Rc::new(ParserConfig::default()),
            diagnostics: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self {
            config:      Rc::new(config),
            diagnostics: Rc::new(RefCell::new(Vec::new())),
        }
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "trace", skip(self), fields(severity = ?diagnostic.severity, code = diagnostic.code.as_deref()))
    )]
    pub fn push_diagnostic(&self, diagnostic: Diagnostic) {
        self.diagnostics.borrow_mut().push(diagnostic);
    }

    /// Push an uncategorised warning. Prefer [`Self::push_warning_code`]
    /// for sites that have a stable diagnostic identifier.
    pub fn push_warning(&self, message: impl Into<String>, span: Option<Span>) {
        let mut d = Diagnostic::warning(message);
        if let Some(s) = span {
            d = d.with_span(s);
        }
        self.push_diagnostic(d);
    }

    /// Push an uncategorised error. Prefer [`Self::push_error_code`].
    pub fn push_error(&self, message: impl Into<String>, span: Option<Span>) {
        let mut d = Diagnostic::error(message);
        if let Some(s) = span {
            d = d.with_span(s);
        }
        self.push_diagnostic(d);
    }

    /// Push a warning with a stable diagnostic code from
    /// [`crate::parse_result::codes`].
    pub fn push_warning_code(
        &self,
        code: &'static str,
        message: impl Into<String>,
        span: Option<Span>,
    ) {
        let mut d = Diagnostic::warning(message).with_code(code);
        if let Some(s) = span {
            d = d.with_span(s);
        }
        self.push_diagnostic(d);
    }

    /// Push an error with a stable diagnostic code from
    /// [`crate::parse_result::codes`].
    pub fn push_error_code(
        &self,
        code: &'static str,
        message: impl Into<String>,
        span: Option<Span>,
    ) {
        let mut d = Diagnostic::error(message).with_code(code);
        if let Some(s) = span {
            d = d.with_span(s);
        }
        self.push_diagnostic(d);
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        Rc::try_unwrap(self.diagnostics)
            .map(RefCell::into_inner)
            .unwrap_or_else(|rc| rc.borrow().clone())
    }

    pub fn snapshot_diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.borrow().clone()
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .borrow()
            .iter()
            .any(|d| matches!(d.severity, Severity::Error))
    }
}

impl Default for ParserState {
    fn default() -> Self {
        Self::new()
    }
}
