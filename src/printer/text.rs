//! Text + MacroRef rendering.

use crate::ast::{BuiltinMacro, ConditionalMacro, MacroKind, MacroRef, Text, TextSegment};

use super::{Printer, TokenKind};

/// Render a `Text` to the printer, escaping literal `%` to `%%`.
pub(crate) fn print_text(p: &mut Printer<'_>, t: &Text) {
    for seg in &t.segments {
        match seg {
            TextSegment::Literal(s) => print_literal_escaped(p, s),
            TextSegment::Macro(m) => print_macro_ref(p, m),
        }
    }
}

/// Emit a [`Text`] as a literal body line: every `TextSegment::Literal`
/// is `%`-escaped (`%` -> `%%`) and emitted under `kind`; every
/// `TextSegment::Macro` is forwarded to [`print_macro_ref`] which
/// classifies macro references on its own.
///
/// Used by the body-rendering paths in `%changelog`, `%description`,
/// and shell-script sections to keep escape logic in one place.
pub(crate) fn print_body_literal_escaped(
    p: &mut super::Printer<'_>,
    t: &crate::ast::Text,
    kind: super::TokenKind,
) {
    use crate::ast::TextSegment;
    for seg in &t.segments {
        match seg {
            TextSegment::Literal(s) => {
                p.emit(kind, &s.replace('%', "%%"));
            }
            TextSegment::Macro(m) => print_macro_ref(p, m),
        }
    }
}

/// Render a literal string segment, escaping `%` to `%%`. Newlines are
/// preserved verbatim — caller is responsible for handling them
/// (typically by splitting and re-emitting with `\` continuations).
pub(crate) fn print_literal_escaped(p: &mut Printer<'_>, s: &str) {
    for c in s.chars() {
        if c == '%' {
            p.raw("%%");
        } else {
            p.raw_char(c);
        }
    }
}

/// Render a [`MacroRef`] with the leading `%` sigil, emitting the
/// whole reference under a single category token so consumers can
/// colour it uniformly. The inner structure is rendered to a side
/// buffer (without classification, since `String: PrintWriter`
/// concatenates) and then passed up as one chunk.
pub(crate) fn print_macro_ref(p: &mut Printer<'_>, m: &MacroRef) {
    let kind = macro_kind_token(&m.kind);
    let mut buf = String::new();
    {
        let mut tmp = Printer::new(&mut buf, p.cfg());
        tmp.raw_char('%');
        print_macro_ref_no_percent(&mut tmp, m);
    }
    p.emit(kind, &buf);
}

/// Map a [`MacroKind`] to the printer's [`TokenKind`] for highlighting.
fn macro_kind_token(kind: &MacroKind) -> TokenKind {
    match kind {
        MacroKind::Shell => TokenKind::ShellMacro,
        MacroKind::Expr | MacroKind::Lua => TokenKind::ExprMacro,
        // `Plain` / `Braced` / `Parametric` / `Builtin` are ordinary
        // macro references; treat them all under `MacroRef`.
        _ => TokenKind::MacroRef,
    }
}

/// Render a macro reference body assuming the leading `%` has already
/// been emitted.
pub(crate) fn print_macro_ref_no_percent(p: &mut Printer<'_>, m: &MacroRef) {
    match &m.kind {
        MacroKind::Plain => {
            print_conditional_prefix(p, m.conditional);
            p.raw(&m.name);
        }
        MacroKind::Braced => {
            p.raw_char('{');
            print_conditional_prefix(p, m.conditional);
            p.raw(&m.name);
            if let Some(value) = &m.with_value {
                p.raw_char(':');
                print_text(p, value);
            }
            p.raw_char('}');
        }
        MacroKind::Parametric => {
            p.raw_char('{');
            print_conditional_prefix(p, m.conditional);
            p.raw(&m.name);
            for arg in &m.args {
                p.raw_char(' ');
                print_text(p, arg);
            }
            p.raw_char('}');
        }
        MacroKind::Shell => {
            p.raw_char('(');
            if let Some(body) = m.args.first() {
                print_text(p, body);
            }
            p.raw_char(')');
        }
        MacroKind::Expr => {
            // `%[…]` when name is empty (bracketed form), else
            // `%{expr:…}` (keyword form).
            if m.name.is_empty() {
                p.raw_char('[');
                if let Some(body) = m.args.first() {
                    print_text(p, body);
                }
                p.raw_char(']');
            } else {
                p.raw_char('{');
                p.raw(&m.name);
                p.raw_char(':');
                if let Some(body) = m.args.first() {
                    print_text(p, body);
                }
                p.raw_char('}');
            }
        }
        MacroKind::Lua => {
            p.raw_char('{');
            // Keyword "lua" — `name` is the literal "lua" in AST.
            let kw = if m.name.is_empty() { "lua" } else { &m.name };
            p.raw(kw);
            p.raw_char(':');
            if let Some(body) = m.args.first() {
                print_text(p, body);
            }
            p.raw_char('}');
        }
        MacroKind::Builtin(b) => {
            p.raw_char('{');
            p.raw(builtin_name(b, &m.name));
            p.raw_char(':');
            if let Some(body) = m.args.first() {
                print_text(p, body);
            }
            p.raw_char('}');
        }
    }
}

fn print_conditional_prefix(p: &mut Printer<'_>, c: ConditionalMacro) {
    match c {
        ConditionalMacro::None => {}
        ConditionalMacro::IfDefined => p.raw_char('?'),
        ConditionalMacro::IfNotDefined => p.raw("!?"),
    }
}

fn builtin_name<'a>(b: &'a BuiltinMacro, fallback_name: &'a str) -> &'a str {
    match b {
        BuiltinMacro::Expand => "expand",
        BuiltinMacro::Expr => "expr",
        BuiltinMacro::Shrink => "shrink",
        BuiltinMacro::Quote => "quote",
        BuiltinMacro::Gsub => "gsub",
        BuiltinMacro::Sub => "sub",
        BuiltinMacro::Len => "len",
        BuiltinMacro::Upper => "upper",
        BuiltinMacro::Lower => "lower",
        BuiltinMacro::Reverse => "reverse",
        BuiltinMacro::Basename => "basename",
        BuiltinMacro::Dirname => "dirname",
        BuiltinMacro::Suffix => "suffix",
        BuiltinMacro::Exists => "exists",
        BuiltinMacro::Load => "load",
        BuiltinMacro::Echo => "echo",
        BuiltinMacro::Warn => "warn",
        BuiltinMacro::Error => "error",
        BuiltinMacro::Dnl => "dnl",
        BuiltinMacro::Trace => "trace",
        BuiltinMacro::Dump => "dump",
        BuiltinMacro::Other(name) => {
            // Use the `Other`'s stored name; fall back to `MacroRef.name`
            // if for some reason it is empty.
            if !name.is_empty() {
                name.as_ref()
            } else {
                fallback_name
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::PrinterConfig;

    fn render(t: &Text) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_text(&mut p, t);
        buf
    }

    fn render_macro(m: &MacroRef) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_macro_ref(&mut p, m);
        buf
    }

    fn macro_named(name: &str, kind: MacroKind) -> MacroRef {
        MacroRef {
            kind,
            name: name.into(),
            args: Vec::new(),
            conditional: ConditionalMacro::None,
            with_value: None,
        }
    }

    #[test]
    fn literal_escapes_percent() {
        let t = Text::from("50%");
        assert_eq!(render(&t), "50%%");
    }

    #[test]
    fn literal_only_no_escape() {
        let t = Text::from("hello");
        assert_eq!(render(&t), "hello");
    }

    #[test]
    fn plain_macro() {
        let m = macro_named("name", MacroKind::Plain);
        assert_eq!(render_macro(&m), "%name");
    }

    #[test]
    fn braced_macro_conditional() {
        let mut m = macro_named("dist", MacroKind::Braced);
        m.conditional = ConditionalMacro::IfDefined;
        assert_eq!(render_macro(&m), "%{?dist}");
    }

    #[test]
    fn braced_with_value() {
        let mut m = macro_named("foo", MacroKind::Braced);
        m.conditional = ConditionalMacro::IfDefined;
        m.with_value = Some(Text::from("value"));
        assert_eq!(render_macro(&m), "%{?foo:value}");
    }

    #[test]
    fn parametric_macro() {
        let mut m = macro_named("foo", MacroKind::Parametric);
        m.args = vec![Text::from("a"), Text::from("b")];
        assert_eq!(render_macro(&m), "%{foo a b}");
    }

    #[test]
    fn shell_macro() {
        let mut m = MacroRef {
            kind: MacroKind::Shell,
            name: String::new(),
            args: vec![Text::from("date +%Y")],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        // The body literal "%" in args will be escaped, so "date +%Y"
        // becomes "date +%%Y".
        let _ = &mut m;
        let r = render_macro(&m);
        assert_eq!(r, "%(date +%%Y)");
    }

    #[test]
    fn expr_brackets() {
        let m = MacroRef {
            kind: MacroKind::Expr,
            name: String::new(),
            args: vec![Text::from("1+1")],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(render_macro(&m), "%[1+1]");
    }

    #[test]
    fn lua_block() {
        let m = MacroRef {
            kind: MacroKind::Lua,
            name: "lua".into(),
            args: vec![Text::from("print('hi')")],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(render_macro(&m), "%{lua:print('hi')}");
    }

    #[test]
    fn builtin_shrink() {
        let m = MacroRef {
            kind: MacroKind::Builtin(BuiltinMacro::Shrink),
            name: "shrink".into(),
            args: vec![Text::from(" a   b ")],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(render_macro(&m), "%{shrink: a   b }");
    }

    #[test]
    fn builtin_other_uses_stored_name() {
        let m = MacroRef {
            kind: MacroKind::Builtin(BuiltinMacro::Other("frobnicate".into())),
            name: "frobnicate".into(),
            args: vec![Text::from("body")],
            conditional: ConditionalMacro::None,
            with_value: None,
        };
        assert_eq!(render_macro(&m), "%{frobnicate:body}");
    }

    #[test]
    fn positional_arg() {
        let m = macro_named("1", MacroKind::Plain);
        assert_eq!(render_macro(&m), "%1");
    }

    #[test]
    fn flag_ref_braced() {
        let m = macro_named("-f", MacroKind::Braced);
        assert_eq!(render_macro(&m), "%{-f}");
    }

    #[test]
    fn mixed_literal_and_macros() {
        let mut t = Text::default();
        t.segments.push(TextSegment::Literal("prefix-".into()));
        t.segments.push(TextSegment::Macro(Box::new(macro_named(
            "name",
            MacroKind::Braced,
        ))));
        t.segments.push(TextSegment::Literal("-tail".into()));
        assert_eq!(render(&t), "prefix-%{name}-tail");
    }
}
