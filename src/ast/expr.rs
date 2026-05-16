//! Parsed `%if` / `%elif` expression tree.
//!
//! The body of a conditional head — everything after the `%if` keyword
//! — follows a small expression grammar that the upstream RPM
//! evaluator interprets at build time. We mirror it here so consumers
//! (linters, formatters) can analyse the expression at the level of
//! operands and operators rather than as opaque text.
//!
//! Parsing is best-effort: when the grammar fails to fit, the
//! containing [`super::cond::CondExpr`] keeps a [`super::cond::CondExpr::Raw`]
//! fallback with the original text. New code should prefer matching
//! on the parsed form and fall back to `Raw` for completeness.
//!
//! Every node carries `data: T` — the parser fills it with a
//! [`super::span::Span`] over the source bytes the node was built
//! from, so callers can produce byte-precise edits and the printer
//! can round-trip exactly.
//!
//! ## Grammar (precedence from low to high)
//!
//! ```text
//! expr     := log_or
//! log_or   := log_and  ('||' log_and)*
//! log_and  := equality ('&&' equality)*
//! equality := relational (('==' | '!=') relational)*
//! relational := unary  (('<=' | '>=' | '<' | '>') unary)*
//! unary    := '!' unary | primary
//! primary  := integer | string | macro | identifier | '(' expr ')'
//! ```
//!
//! Arithmetic operators (`+`, `-`, `*`, `/`) are intentionally outside
//! the modelled subset; expressions containing them fall through to
//! [`super::cond::CondExpr::Raw`].

/// Binary operator slot. Discriminates the [`ExprAst::Binary`] variant.
///
/// This set is closed — RPM's `%if` grammar does not grow new operators
/// in practice. The enum is therefore *not* `non_exhaustive`, so
/// downstream code can match it exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub enum BinOp {
    /// `||` — logical OR (eager; RPM does not short-circuit).
    LogOr,
    /// `&&` — logical AND (eager).
    LogAnd,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

impl BinOp {
    /// Canonical source rendering of the operator, with the spaces
    /// callers typically want around it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            BinOp::LogOr => "||",
            BinOp::LogAnd => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
        }
    }
}

/// Parsed `%if` / `%elif` expression tree.
///
/// See the [module docs](self) for the grammar and parsing rules.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub enum ExprAst<T = ()> {
    /// Integer literal: `0`, `1`, `120000`. The grammar has no unary
    /// minus — `-1` will fail to parse and the surrounding
    /// [`super::cond::CondExpr`] falls through to
    /// [`super::cond::CondExpr::Raw`].
    Integer {
        /// Numeric value, parsed as `i64`. Values that overflow `i64`
        /// cause the surrounding `CondExpr` to fall back to `Raw`.
        value: i64,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// Quoted string literal: `"foo"`, `"%{_vendor}"`. The value does
    /// *not* include the surrounding quotes; the printer re-emits ASCII
    /// `"` around `value` literally and does **not** escape inner `"`.
    /// Embedded `%{…}` is preserved as part of `value` as a flat
    /// `String` — it is *not* split into `Text` segments, so callers
    /// that need to evaluate it must invoke macro expansion separately.
    String {
        /// Verbatim string contents without the surrounding `"`.
        value: String,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// `%{name}` / `%{?name}` macro reference, stored verbatim
    /// (including the leading `%` and any braces). The text is kept as
    /// a flat `String` rather than decomposed into `TextSegment` —
    /// downstream consumers that walk macro segments should treat the
    /// `Macro` variant itself as the signal, not look for
    /// [`super::text::TextSegment::Macro`] inside.
    Macro {
        /// Verbatim macro reference, as written in source.
        text: String,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// Bare identifier — an unquoted token that is neither a keyword
    /// nor a number. RPM's evaluator typically rejects these; the
    /// grammar accepts them so the surrounding parse doesn't bail out
    /// on suspicious input.
    ///
    /// **Lint-signal variant:** consumers should treat `Identifier` as
    /// a hint that the source looks dubious, *not* as a validated RPM
    /// token. Static analysers built on top of this AST are expected
    /// to flag (or normalise) it.
    Identifier {
        /// Identifier text.
        name: String,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// `(expr)` — parens are preserved as a node so the printer can
    /// round-trip exactly. Analysis can flatten via
    /// [`ExprAst::peel_parens`].
    Paren {
        /// The grouped sub-expression.
        inner: Box<ExprAst<T>>,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// `!expr` — logical NOT.
    Not {
        /// The negated sub-expression.
        inner: Box<ExprAst<T>>,
        /// Per-node user-data (typically a span).
        data: T,
    },
    /// Binary operator application. Operator precedence is encoded in
    /// the tree shape: lower-precedence operators sit higher up.
    Binary {
        /// Operator slot.
        kind: BinOp,
        /// Left-hand operand.
        lhs: Box<ExprAst<T>>,
        /// Right-hand operand.
        rhs: Box<ExprAst<T>>,
        /// Per-node user-data (typically a span).
        data: T,
    },
}

impl<T> ExprAst<T> {
    /// Per-node user-data (typically a [`super::span::Span`]). Returned
    /// by reference so the caller decides whether to clone.
    pub fn data(&self) -> &T {
        match self {
            ExprAst::Integer { data, .. }
            | ExprAst::String { data, .. }
            | ExprAst::Macro { data, .. }
            | ExprAst::Identifier { data, .. }
            | ExprAst::Paren { data, .. }
            | ExprAst::Not { data, .. }
            | ExprAst::Binary { data, .. } => data,
        }
    }

    /// Strip surrounding [`ExprAst::Paren`] wrappers, returning the
    /// inner expression. Useful for analysis that doesn't care about
    /// grouping syntax (e.g. tautology detection).
    ///
    /// # Examples
    ///
    /// ```
    /// use rpm_spec::ast::ExprAst;
    /// let inner = ExprAst::Integer { value: 1, data: () };
    /// let wrapped = ExprAst::Paren {
    ///     inner: Box::new(ExprAst::Paren {
    ///         inner: Box::new(inner.clone()),
    ///         data: (),
    ///     }),
    ///     data: (),
    /// };
    /// assert_eq!(wrapped.peel_parens(), &inner);
    /// ```
    #[must_use]
    pub fn peel_parens(&self) -> &Self {
        let mut current = self;
        while let ExprAst::Paren { inner, .. } = current {
            current = inner;
        }
        current
    }
}
