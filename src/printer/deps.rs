//! Dependency expression rendering.

use crate::ast::{BoolDep, DepAtom, DepExpr, EVR, VerOp};

use super::Printer;
use super::text::print_text;

pub(crate) fn print_dep_expr(p: &mut Printer<'_>, e: &DepExpr) {
    match e {
        DepExpr::Atom(a) => print_dep_atom(p, a),
        DepExpr::Rich(b) => print_rich(p, b),
    }
}

fn print_dep_atom(p: &mut Printer<'_>, a: &DepAtom) {
    print_text(p, &a.name);
    if let Some(arch) = &a.arch {
        p.raw_char('(');
        print_text(p, arch);
        p.raw_char(')');
    }
    if let Some(c) = &a.constraint {
        p.raw_char(' ');
        p.raw(op_str(c.op));
        p.raw_char(' ');
        print_evr(p, &c.evr);
    }
}

fn print_evr(p: &mut Printer<'_>, evr: &EVR) {
    if let Some(epoch) = evr.epoch {
        p.raw(&epoch.to_string());
        p.raw_char(':');
    }
    print_text(p, &evr.version);
    if let Some(rel) = &evr.release {
        p.raw_char('-');
        print_text(p, rel);
    }
}

fn op_str(op: VerOp) -> &'static str {
    match op {
        VerOp::Lt => "<",
        VerOp::Le => "<=",
        VerOp::Eq => "=",
        VerOp::Ne => "!=",
        VerOp::Ge => ">=",
        VerOp::Gt => ">",
    }
}

fn print_rich(p: &mut Printer<'_>, b: &BoolDep) {
    p.raw_char('(');
    match b {
        BoolDep::And(xs) => print_operator_list(p, xs, "and"),
        BoolDep::Or(xs) => print_operator_list(p, xs, "or"),
        BoolDep::With(xs) => print_operator_list(p, xs, "with"),
        BoolDep::Without { left, right } => {
            print_dep_expr(p, left);
            p.raw(" without ");
            print_dep_expr(p, right);
        }
        BoolDep::If {
            cond,
            then,
            otherwise,
        } => {
            print_dep_expr(p, then);
            p.raw(" if ");
            print_dep_expr(p, cond);
            if let Some(o) = otherwise {
                p.raw(" else ");
                print_dep_expr(p, o);
            }
        }
        BoolDep::Unless {
            cond,
            then,
            otherwise,
        } => {
            print_dep_expr(p, then);
            p.raw(" unless ");
            print_dep_expr(p, cond);
            if let Some(o) = otherwise {
                p.raw(" else ");
                print_dep_expr(p, o);
            }
        }
    }
    p.raw_char(')');
}

fn print_operator_list(p: &mut Printer<'_>, xs: &[DepExpr], op: &str) {
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            p.raw_char(' ');
            p.raw(op);
            p.raw_char(' ');
        }
        print_dep_expr(p, x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Text;
    use crate::printer::PrinterConfig;

    fn render(e: &DepExpr) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_dep_expr(&mut p, e);
        buf
    }

    #[test]
    fn atom_bare() {
        let a = DepAtom {
            name: Text::from("glibc"),
            arch: None,
            constraint: None,
        };
        assert_eq!(render(&DepExpr::Atom(a)), "glibc");
    }

    #[test]
    fn atom_with_version() {
        let a = DepAtom {
            name: Text::from("glibc"),
            arch: None,
            constraint: Some(crate::ast::DepConstraint {
                op: VerOp::Ge,
                evr: EVR {
                    epoch: None,
                    version: Text::from("2.34"),
                    release: None,
                },
            }),
        };
        assert_eq!(render(&DepExpr::Atom(a)), "glibc >= 2.34");
    }

    #[test]
    fn atom_with_epoch_release_arch() {
        let a = DepAtom {
            name: Text::from("perl-DBI"),
            arch: Some(Text::from("x86-64")),
            constraint: Some(crate::ast::DepConstraint {
                op: VerOp::Eq,
                evr: EVR {
                    epoch: Some(9),
                    version: Text::from("1.643"),
                    release: Some(Text::from("1.fc40")),
                },
            }),
        };
        assert_eq!(
            render(&DepExpr::Atom(a)),
            "perl-DBI(x86-64) = 9:1.643-1.fc40"
        );
    }

    #[test]
    fn rich_and() {
        let e = DepExpr::Rich(Box::new(BoolDep::And(vec![
            DepExpr::Atom(DepAtom {
                name: Text::from("a"),
                arch: None,
                constraint: None,
            }),
            DepExpr::Atom(DepAtom {
                name: Text::from("b"),
                arch: None,
                constraint: None,
            }),
        ])));
        assert_eq!(render(&e), "(a and b)");
    }

    #[test]
    fn rich_if_else() {
        let e = DepExpr::Rich(Box::new(BoolDep::If {
            cond: Box::new(DepExpr::Atom(DepAtom {
                name: Text::from("cond"),
                arch: None,
                constraint: None,
            })),
            then: Box::new(DepExpr::Atom(DepAtom {
                name: Text::from("then"),
                arch: None,
                constraint: None,
            })),
            otherwise: Some(Box::new(DepExpr::Atom(DepAtom {
                name: Text::from("other"),
                arch: None,
                constraint: None,
            }))),
        }));
        assert_eq!(render(&e), "(then if cond else other)");
    }

    #[test]
    fn rich_without() {
        let e = DepExpr::Rich(Box::new(BoolDep::Without {
            left: Box::new(DepExpr::Atom(DepAtom {
                name: Text::from("a"),
                arch: None,
                constraint: None,
            })),
            right: Box::new(DepExpr::Atom(DepAtom {
                name: Text::from("b"),
                arch: None,
                constraint: None,
            })),
        }));
        assert_eq!(render(&e), "(a without b)");
    }

    #[test]
    fn rich_nested() {
        let inner = DepExpr::Rich(Box::new(BoolDep::And(vec![
            DepExpr::Atom(DepAtom {
                name: Text::from("a"),
                arch: None,
                constraint: None,
            }),
            DepExpr::Atom(DepAtom {
                name: Text::from("b"),
                arch: None,
                constraint: None,
            }),
        ])));
        let outer = DepExpr::Rich(Box::new(BoolDep::Or(vec![
            inner,
            DepExpr::Atom(DepAtom {
                name: Text::from("c"),
                arch: None,
                constraint: None,
            }),
        ])));
        assert_eq!(render(&outer), "((a and b) or c)");
    }
}
