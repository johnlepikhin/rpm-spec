//! Render a parsed `%if` / `%elif` expression tree back to source.
//!
//! The output uses the canonical surface form (`a && b`, `(expr)`, …)
//! — exact round-trip with the original whitespace requires the source
//! bytes, which [`super::Printer`] doesn't carry. Callers that need
//! byte-identical output should keep the input source around and slice
//! it themselves.

use crate::ast::ExprAst;

use super::Printer;

/// Render an [`ExprAst`] node and its sub-tree.
pub(crate) fn print_expr_ast<T>(p: &mut Printer<'_>, ast: &ExprAst<T>) {
    match ast {
        ExprAst::Integer { value, .. } => p.raw(&value.to_string()),
        ExprAst::String { value, .. } => {
            p.raw_char('"');
            p.raw(value);
            p.raw_char('"');
        }
        ExprAst::Macro { text, .. } => p.raw(text),
        ExprAst::Identifier { name, .. } => p.raw(name),
        ExprAst::Paren { inner, .. } => {
            p.raw_char('(');
            print_expr_ast(p, inner);
            p.raw_char(')');
        }
        ExprAst::Not { inner, .. } => {
            p.raw_char('!');
            print_expr_ast(p, inner);
        }
        ExprAst::Binary { kind, lhs, rhs, .. } => {
            print_expr_ast(p, lhs);
            p.raw_char(' ');
            p.raw(kind.as_str());
            p.raw_char(' ');
            print_expr_ast(p, rhs);
        }
    }
}
