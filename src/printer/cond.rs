//! Conditional block rendering (with optional indentation).

use crate::ast::{CondExpr, CondKind, Conditional, Text, TextSegment};

use super::Printer;
use super::expr::print_expr_ast;
use super::text::{print_macro_ref, print_text};

/// Render a [`Conditional`] block. `body_printer` is called for every
/// body item; callers pass the appropriate `print_*_content` function
/// matching the body type.
pub(crate) fn print_conditional<T, Body, F>(
    p: &mut Printer<'_>,
    c: &Conditional<T, Body>,
    body_printer: F,
) where
    F: Fn(&mut Printer<'_>, &Body),
{
    for (i, branch) in c.branches.iter().enumerate() {
        let head_kw = if i == 0 {
            kind_keyword_head(branch.kind)
        } else {
            kind_keyword_elif(branch.kind)
        };
        emit_branch_head(p, head_kw, &branch.expr);
        p.nested(|p| {
            for item in &branch.body {
                body_printer(p, item);
            }
        });
    }
    if let Some(body) = &c.otherwise {
        p.write_indent();
        p.raw("%else");
        p.newline();
        p.nested(|p| {
            for item in body {
                body_printer(p, item);
            }
        });
    }
    p.write_indent();
    p.raw("%endif");
    p.newline();
}

fn emit_branch_head<T>(p: &mut Printer<'_>, kw: &str, expr: &CondExpr<T>) {
    p.write_indent();
    p.raw(kw);
    match expr {
        CondExpr::Raw(t) => {
            if !is_empty_text(t) {
                p.raw_char(' ');
                // `%if` / `%elif` expressions are stored as raw source
                // text — the parser keeps `0%{?fedora}` as a single
                // literal rather than decomposing into Macro segments.
                // We therefore emit it verbatim, *without* escaping `%`
                // to `%%`.
                print_raw_cond_text(p, t);
            }
        }
        CondExpr::Parsed(ast) => {
            p.raw_char(' ');
            print_expr_ast(p, ast);
        }
        CondExpr::ArchList(items) => {
            for item in items {
                p.raw_char(' ');
                print_text(p, item);
            }
        }
    }
    p.newline();
}

/// Render a [`Text`] without `%%`-escaping literal `%`. Used for
/// `CondExpr::Raw` where the parser stored the source-form text
/// intact.
fn print_raw_cond_text(p: &mut Printer<'_>, t: &Text) {
    for seg in &t.segments {
        match seg {
            TextSegment::Literal(s) => p.raw(s),
            TextSegment::Macro(m) => print_macro_ref(p, m),
        }
    }
}

fn is_empty_text(t: &crate::ast::Text) -> bool {
    t.segments.iter().all(|s| match s {
        crate::ast::TextSegment::Literal(l) => l.is_empty(),
        crate::ast::TextSegment::Macro(_) => false,
    })
}

fn kind_keyword_head(k: CondKind) -> &'static str {
    match k {
        CondKind::If => "%if",
        CondKind::IfArch => "%ifarch",
        CondKind::IfNArch => "%ifnarch",
        CondKind::IfOs => "%ifos",
        CondKind::IfNOs => "%ifnos",
        // Should not appear as a branch head; tolerate by emitting %if.
        CondKind::Elif | CondKind::ElifArch | CondKind::ElifOs => "%if",
    }
}

fn kind_keyword_elif(k: CondKind) -> &'static str {
    match k {
        CondKind::Elif => "%elif",
        CondKind::ElifArch => "%elifarch",
        CondKind::ElifOs => "%elifos",
        // Tolerate misuse: a non-elif kind in a non-head position is
        // emitted as a plain %elif.
        _ => "%elif",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{CondBranch, Text, TextSegment};
    use crate::printer::PrinterConfig;

    fn render<F>(c: &Conditional<(), &'static str>, cfg: &PrinterConfig, body: F) -> String
    where
        F: Fn(&mut Printer<'_>, &&'static str),
    {
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, cfg);
        print_conditional(&mut p, c, body);
        buf
    }

    fn lit(s: &str) -> Text {
        Text {
            segments: vec![TextSegment::Literal(s.into())],
        }
    }

    fn simple_cond() -> Conditional<(), &'static str> {
        Conditional {
            branches: vec![CondBranch {
                kind: CondKind::If,
                expr: CondExpr::Raw(lit("foo")),
                body: vec!["A", "B"],
                data: (),
            }],
            otherwise: None,
            data: (),
        }
    }

    #[test]
    fn flat_indent_zero() {
        let cfg = PrinterConfig::default();
        let out = render(&simple_cond(), &cfg, |p, s| {
            p.write_indent();
            p.raw(s);
            p.newline();
        });
        assert_eq!(out, "%if foo\nA\nB\n%endif\n");
    }

    #[test]
    fn indent_two_spaces() {
        let cfg = PrinterConfig::new().with_indent(2);
        let out = render(&simple_cond(), &cfg, |p, s| {
            p.write_indent();
            p.raw(s);
            p.newline();
        });
        assert_eq!(out, "%if foo\n  A\n  B\n%endif\n");
    }

    #[test]
    fn elif_else() {
        let c = Conditional {
            branches: vec![
                CondBranch {
                    kind: CondKind::If,
                    expr: CondExpr::Raw(lit("1")),
                    body: vec!["A"],
                    data: (),
                },
                CondBranch {
                    kind: CondKind::Elif,
                    expr: CondExpr::Raw(lit("2")),
                    body: vec!["B"],
                    data: (),
                },
            ],
            otherwise: Some(vec!["C"]),
            data: (),
        };
        let cfg = PrinterConfig::default();
        let out = render(&c, &cfg, |p, s| {
            p.write_indent();
            p.raw(s);
            p.newline();
        });
        assert_eq!(out, "%if 1\nA\n%elif 2\nB\n%else\nC\n%endif\n");
    }

    #[test]
    fn ifarch_with_list() {
        let c = Conditional {
            branches: vec![CondBranch {
                kind: CondKind::IfArch,
                expr: CondExpr::ArchList(vec![lit("x86_64"), lit("aarch64")]),
                body: vec!["X"],
                data: (),
            }],
            otherwise: None,
            data: (),
        };
        let cfg = PrinterConfig::default();
        let out = render(&c, &cfg, |p, s| {
            p.write_indent();
            p.raw(s);
            p.newline();
        });
        assert_eq!(out, "%ifarch x86_64 aarch64\nX\n%endif\n");
    }
}
