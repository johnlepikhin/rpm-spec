//! Render a parsed `%if` / `%elif` expression tree back to source.
//!
//! The output uses the canonical surface form (`a && b`, `(expr)`, …)
//! — exact round-trip with the original whitespace requires the source
//! bytes, which [`super::Printer`] doesn't carry. Callers that need
//! byte-identical output should keep the input source around and slice
//! it themselves.

use crate::ast::ExprAst;

use super::{Printer, TokenKind};

/// Render an [`ExprAst`] node and its sub-tree.
pub(crate) fn print_expr_ast<T>(p: &mut Printer<'_>, ast: &ExprAst<T>) {
    match ast {
        ExprAst::Integer { value, .. } => p.emit(TokenKind::Number, &value.to_string()),
        ExprAst::String { value, .. } => {
            // Emit the opening/closing quotes as part of the String
            // token so consumers can render the whole literal in one
            // colour. The inner `value` doesn't carry source-level
            // escape information — emit verbatim.
            let mut buf = String::with_capacity(value.len() + 2);
            buf.push('"');
            buf.push_str(value);
            buf.push('"');
            p.emit(TokenKind::String, &buf);
        }
        ExprAst::Macro { text, .. } => p.emit(TokenKind::MacroRef, text),
        ExprAst::Identifier { name, .. } => p.raw(name),
        ExprAst::Paren { inner, .. } => {
            p.raw_char('(');
            print_expr_ast(p, inner);
            p.raw_char(')');
        }
        ExprAst::Not { inner, .. } => {
            p.emit(TokenKind::Operator, "!");
            print_expr_ast(p, inner);
        }
        ExprAst::Binary { kind, lhs, rhs, .. } => {
            print_expr_ast(p, lhs);
            p.raw_char(' ');
            p.emit(TokenKind::Operator, kind.as_str());
            p.raw_char(' ');
            print_expr_ast(p, rhs);
        }
    }
}
