//! Top-level macro statements + comments rendering.

use crate::ast::{
    BuildCondStyle, BuildCondition, Comment, CommentStyle, IncludeDirective, MacroDef,
    MacroDefKind, Text, TextSegment,
};

use super::text::print_text;
use super::{Printer, TokenKind};

pub(crate) fn print_macro_def<T>(p: &mut Printer<'_>, m: &MacroDef<T>) {
    p.write_indent();
    let kw = match m.kind {
        MacroDefKind::Define => "%define",
        MacroDefKind::Global => "%global",
        MacroDefKind::Undefine => "%undefine",
    };
    p.emit(TokenKind::MacroDefKeyword, kw);
    p.raw_char(' ');
    p.emit(TokenKind::MacroRef, &m.name);
    if let Some(opts) = &m.opts {
        p.emit(TokenKind::Flag, opts);
    }
    if matches!(m.kind, MacroDefKind::Undefine) {
        // No body for %undefine.
        p.newline();
        return;
    }
    if !is_empty_text(&m.body) {
        p.raw_char(' ');
        print_body_with_continuations(p, &m.body);
    }
    p.newline();
}

/// Render a macro body that may span several source lines and re-emit
/// each `\n` boundary as a trailing ` \` continuation.
///
/// The body is rendered into a side buffer (with `%%`-escaping) and
/// then split on `\n`. Caveat: a macro whose own rendering contains a
/// literal `\n` in one of its arguments will be split across the line
/// break, which is almost certainly wrong — but our parser never
/// produces such an AST (a multi-line `%define` body is decomposed
/// into `Literal` segments at parse time, with macros embedded between
/// them).
fn print_body_with_continuations(p: &mut Printer<'_>, body: &Text) {
    let mut buf = String::new();
    {
        let mut tmp = Printer::new(&mut buf, p.cfg());
        print_text(&mut tmp, body);
    }
    let mut lines = buf.split('\n');
    if let Some(first) = lines.next() {
        p.raw(first);
    }
    for line in lines {
        p.raw(" \\");
        p.newline();
        p.write_indent();
        p.raw(line);
    }
}

fn is_empty_text(t: &Text) -> bool {
    t.segments.iter().all(|s| match s {
        TextSegment::Literal(l) => l.is_empty(),
        TextSegment::Macro(_) => false,
    })
}

pub(crate) fn print_build_condition<T>(p: &mut Printer<'_>, b: &BuildCondition<T>) {
    p.write_indent();
    let kw = match b.style {
        BuildCondStyle::Bcond => "%bcond",
        BuildCondStyle::BcondWith => "%bcond_with",
        BuildCondStyle::BcondWithout => "%bcond_without",
    };
    p.emit(TokenKind::MacroDefKeyword, kw);
    p.raw_char(' ');
    p.emit(TokenKind::MacroRef, &b.name);
    if let Some(default) = &b.default {
        p.raw_char(' ');
        print_text(p, default);
    }
    p.newline();
}

pub(crate) fn print_include<T>(p: &mut Printer<'_>, i: &IncludeDirective<T>) {
    p.write_indent();
    p.emit(TokenKind::MacroDefKeyword, "%include");
    p.raw_char(' ');
    print_text(p, &i.path);
    p.newline();
}

pub(crate) fn print_comment<T>(p: &mut Printer<'_>, c: &Comment<T>) {
    p.write_indent();
    // Hash-style comments are pure plain text; `%dnl` is a comment
    // directive but historically rendered uniformly under `Comment`.
    let prefix = match c.style {
        CommentStyle::Hash => "#",
        CommentStyle::Dnl => "%dnl",
    };
    p.emit(TokenKind::Comment, prefix);
    if !is_empty_text(&c.text) {
        // Inline the comment body as plain text inside the Comment
        // token so consumers can colour the whole line uniformly.
        let mut buf = String::new();
        {
            let mut tmp = Printer::new(&mut buf, p.cfg());
            tmp.raw_char(' ');
            print_text(&mut tmp, &c.text);
        }
        p.emit(TokenKind::Comment, &buf);
    }
    p.newline();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::PrinterConfig;

    fn render<F: FnOnce(&mut Printer<'_>)>(f: F) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        f(&mut p);
        buf
    }

    #[test]
    fn define_simple() {
        let m: MacroDef<()> = MacroDef {
            kind: MacroDefKind::Define,
            name: "foo".into(),
            opts: None,
            body: Text::from("bar"),
            eager: false,
            global: false,
            literal: false,
            one_shot: false,
            data: (),
        };
        assert_eq!(render(|p| print_macro_def(p, &m)), "%define foo bar\n");
    }

    #[test]
    fn global_with_opts() {
        let m: MacroDef<()> = MacroDef {
            kind: MacroDefKind::Global,
            name: "greet".into(),
            opts: Some("(n:)".into()),
            body: Text::from("Hello"),
            eager: false,
            global: true,
            literal: false,
            one_shot: false,
            data: (),
        };
        assert_eq!(render(|p| print_macro_def(p, &m)), "%global greet(n:) Hello\n");
    }

    #[test]
    fn define_multiline_body() {
        let m: MacroDef<()> = MacroDef {
            kind: MacroDefKind::Define,
            name: "foo".into(),
            opts: None,
            body: Text::from("a\nb\nc"),
            eager: false,
            global: false,
            literal: false,
            one_shot: false,
            data: (),
        };
        assert_eq!(
            render(|p| print_macro_def(p, &m)),
            "%define foo a \\\nb \\\nc\n"
        );
    }

    #[test]
    fn undefine() {
        let m: MacroDef<()> = MacroDef {
            kind: MacroDefKind::Undefine,
            name: "foo".into(),
            opts: None,
            body: Text::new(),
            eager: false,
            global: false,
            literal: false,
            one_shot: false,
            data: (),
        };
        assert_eq!(render(|p| print_macro_def(p, &m)), "%undefine foo\n");
    }

    #[test]
    fn bcond_with_default() {
        let b: BuildCondition<()> = BuildCondition {
            style: BuildCondStyle::Bcond,
            name: "openssl".into(),
            default: Some(Text::from("1")),
            data: (),
        };
        assert_eq!(render(|p| print_build_condition(p, &b)), "%bcond openssl 1\n");
    }

    #[test]
    fn bcond_with_no_default() {
        let b: BuildCondition<()> = BuildCondition {
            style: BuildCondStyle::BcondWith,
            name: "ssl".into(),
            default: None,
            data: (),
        };
        assert_eq!(render(|p| print_build_condition(p, &b)), "%bcond_with ssl\n");
    }

    #[test]
    fn include_directive() {
        let i: IncludeDirective<()> = IncludeDirective {
            path: Text::from("/etc/rpm/macros.fragment"),
            data: (),
        };
        assert_eq!(
            render(|p| print_include(p, &i)),
            "%include /etc/rpm/macros.fragment\n"
        );
    }

    #[test]
    fn hash_comment() {
        let c: Comment<()> = Comment {
            style: CommentStyle::Hash,
            text: Text::from("workaround for bug #42"),
            data: (),
        };
        assert_eq!(
            render(|p| print_comment(p, &c)),
            "# workaround for bug #42\n"
        );
    }

    #[test]
    fn dnl_comment() {
        let c: Comment<()> = Comment {
            style: CommentStyle::Dnl,
            text: Text::from("this is invisible"),
            data: (),
        };
        assert_eq!(render(|p| print_comment(p, &c)), "%dnl this is invisible\n");
    }
}
