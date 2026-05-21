//! Scriptlet / trigger / file-trigger rendering.

use crate::ast::{
    FileTrigger, FileTriggerKind, Interpreter, Scriptlet, ScriptletKind, Trigger, TriggerKind,
};

use super::deps::print_dep_expr;
use super::text::print_text;
use super::util::print_subpkg;
use super::{Printer, TokenKind};

pub(crate) fn print_scriptlet<T>(p: &mut Printer<'_>, s: &Scriptlet<T>) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, scriptlet_keyword(s.kind));
    print_subpkg(p, s.subpkg.as_ref());
    print_interp(p, s.interp.as_ref());
    if s.expand_macros {
        p.raw(" -e");
    }
    if s.quiet {
        p.raw(" -q");
    }
    if let Some(file) = &s.from_file {
        p.raw(" -f ");
        print_text(p, file);
    }
    p.newline();
    print_shell_body(p, &s.body);
}

pub(crate) fn print_trigger<T>(p: &mut Printer<'_>, t: &Trigger<T>) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, trigger_keyword(t.kind));
    print_subpkg(p, t.subpkg.as_ref());
    print_interp(p, t.interp.as_ref());
    if !t.conditions.is_empty() {
        p.raw(" --");
        for (i, c) in t.conditions.iter().enumerate() {
            if i == 0 {
                p.raw_char(' ');
            } else {
                p.raw(", ");
            }
            print_dep_expr(p, c);
        }
    }
    p.newline();
    print_shell_body(p, &t.body);
}

pub(crate) fn print_file_trigger<T>(p: &mut Printer<'_>, ft: &FileTrigger<T>) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, file_trigger_keyword(ft.kind));
    print_subpkg(p, ft.subpkg.as_ref());
    print_interp(p, ft.interp.as_ref());
    if let Some(prio) = ft.priority {
        p.raw(" -P ");
        p.raw(&prio.to_string());
    }
    if !ft.prefixes.is_empty() {
        p.raw(" --");
        for (i, prefix) in ft.prefixes.iter().enumerate() {
            if i == 0 {
                p.raw_char(' ');
            } else {
                p.raw(", ");
            }
            print_text(p, prefix);
        }
    }
    p.newline();
    print_shell_body(p, &ft.body);
}

fn print_interp(p: &mut Printer<'_>, interp: Option<&Interpreter>) {
    match interp {
        Some(Interpreter::Lua) => p.raw(" -p <lua>"),
        Some(Interpreter::Path(path)) => {
            p.raw(" -p ");
            print_text(p, path);
        }
        None => {}
    }
}

fn print_shell_body<T>(p: &mut Printer<'_>, body: &crate::ast::ShellBody<T>) {
    for line in &body.lines {
        p.write_indent();
        print_text_trim_leading_ws(p, line);
        p.newline();
    }
}

/// Render a body line, stripping any leading whitespace from its first
/// literal segment so the printer is the sole source of indentation.
///
/// Background: the parser tolerates (and preserves) leading whitespace
/// before macro statements that live inside a `%if`. Without trimming,
/// a `parse → print(indent=N) → parse → print(indent=N)` cycle would
/// cumulatively double the indent on every pass: the literal whitespace
/// captured by the parser would be re-emitted in addition to the
/// printer's own `write_indent()`. Trimming here keeps the printer
/// idempotent — see `tests/scriptlet_indent.rs`.
fn print_text_trim_leading_ws(p: &mut Printer<'_>, t: &crate::ast::Text) {
    use crate::ast::TextSegment;
    let mut trimmed = false;
    for seg in &t.segments {
        match seg {
            TextSegment::Literal(s) if !trimmed => {
                let stripped = s.trim_start_matches([' ', '\t']);
                if !stripped.is_empty() {
                    super::text::print_literal_escaped(p, stripped);
                    trimmed = true;
                }
                // If `stripped` is empty the whole segment was whitespace;
                // drop it but keep `trimmed = false` so the next literal
                // segment is also trimmed (handles rare segmentations).
            }
            TextSegment::Literal(s) => super::text::print_literal_escaped(p, s),
            TextSegment::Macro(m) => {
                super::text::print_macro_ref(p, m);
                trimmed = true;
            }
        }
    }
}

fn scriptlet_keyword(k: ScriptletKind) -> &'static str {
    match k {
        ScriptletKind::Pre => "%pre",
        ScriptletKind::Post => "%post",
        ScriptletKind::Preun => "%preun",
        ScriptletKind::Postun => "%postun",
        ScriptletKind::Pretrans => "%pretrans",
        ScriptletKind::Posttrans => "%posttrans",
        ScriptletKind::Preuntrans => "%preuntrans",
        ScriptletKind::Postuntrans => "%postuntrans",
    }
}

fn trigger_keyword(k: TriggerKind) -> &'static str {
    match k {
        TriggerKind::Prein => "%triggerprein",
        TriggerKind::In => "%triggerin",
        TriggerKind::Un => "%triggerun",
        TriggerKind::Postun => "%triggerpostun",
    }
}

fn file_trigger_keyword(k: FileTriggerKind) -> &'static str {
    match k {
        FileTriggerKind::In => "%filetriggerin",
        FileTriggerKind::Un => "%filetriggerun",
        FileTriggerKind::Postun => "%filetriggerpostun",
        FileTriggerKind::TransIn => "%transfiletriggerin",
        FileTriggerKind::TransUn => "%transfiletriggerun",
        FileTriggerKind::TransPostun => "%transfiletriggerpostun",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{DepAtom, DepExpr, ShellBody, SubpkgRef, Text};
    use crate::printer::PrinterConfig;

    fn render(s: &Scriptlet<()>) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_scriptlet(&mut p, s);
        buf
    }

    fn render_trigger(t: &Trigger<()>) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_trigger(&mut p, t);
        buf
    }

    fn render_ft(ft: &FileTrigger<()>) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_file_trigger(&mut p, ft);
        buf
    }

    #[test]
    fn post_bare() {
        let s = Scriptlet {
            kind: ScriptletKind::Post,
            subpkg: None,
            interp: None,
            expand_macros: false,
            quiet: false,
            from_file: None,
            body: ShellBody {
                conditionals: Vec::new(),
                lines: vec![Text::from("echo hi")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%post\necho hi\n");
    }

    #[test]
    fn post_with_path_interp() {
        let s = Scriptlet {
            kind: ScriptletKind::Post,
            subpkg: None,
            interp: Some(Interpreter::Path(Text::from("/sbin/ldconfig"))),
            expand_macros: false,
            quiet: false,
            from_file: None,
            body: ShellBody { lines: Vec::new(), conditionals: Vec::new() },
            data: (),
        };
        assert_eq!(render(&s), "%post -p /sbin/ldconfig\n");
    }

    #[test]
    fn post_with_lua() {
        let s = Scriptlet {
            kind: ScriptletKind::Post,
            subpkg: None,
            interp: Some(Interpreter::Lua),
            expand_macros: false,
            quiet: false,
            from_file: None,
            body: ShellBody {
                conditionals: Vec::new(),
                lines: vec![Text::from("print('hi')")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%post -p <lua>\nprint('hi')\n");
    }

    #[test]
    fn post_bare_subpkg() {
        let s = Scriptlet {
            kind: ScriptletKind::Post,
            subpkg: Some(SubpkgRef::Relative(Text::from("libfoo"))),
            interp: None,
            expand_macros: false,
            quiet: false,
            from_file: None,
            body: ShellBody {
                conditionals: Vec::new(),
                lines: vec![Text::from("echo")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%post libfoo\necho\n");
    }

    #[test]
    fn post_absolute_subpkg() {
        let s = Scriptlet {
            kind: ScriptletKind::Post,
            subpkg: Some(SubpkgRef::Absolute(Text::from("libfoo"))),
            interp: None,
            expand_macros: false,
            quiet: false,
            from_file: None,
            body: ShellBody { lines: Vec::new(), conditionals: Vec::new() },
            data: (),
        };
        assert_eq!(render(&s), "%post -n libfoo\n");
    }

    #[test]
    fn trigger_in_with_conditions() {
        let t = Trigger {
            kind: TriggerKind::In,
            subpkg: None,
            interp: None,
            conditions: vec![
                DepExpr::Atom(DepAtom {
                    name: Text::from("foo"),
                    arch: None,
                    constraint: None,
                }),
                DepExpr::Atom(DepAtom {
                    name: Text::from("bar"),
                    arch: None,
                    constraint: None,
                }),
            ],
            body: ShellBody {
                conditionals: Vec::new(),
                lines: vec![Text::from("do-it")],
            },
            data: (),
        };
        assert_eq!(render_trigger(&t), "%triggerin -- foo, bar\ndo-it\n");
    }

    #[test]
    fn file_trigger_with_priority() {
        let ft = FileTrigger {
            kind: FileTriggerKind::In,
            subpkg: None,
            interp: None,
            priority: Some(200),
            prefixes: vec![Text::from("/usr/lib")],
            body: ShellBody {
                conditionals: Vec::new(),
                lines: vec![Text::from("act")],
            },
            data: (),
        };
        assert_eq!(render_ft(&ft), "%filetriggerin -P 200 -- /usr/lib\nact\n");
    }
}
