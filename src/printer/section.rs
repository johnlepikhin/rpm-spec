//! Section dispatch + headers for simple sections.

use crate::ast::{
    BuildScriptKind, PackageName, PreambleContent, Section, ShellBody, SubpkgRef, Text, TextBody,
};

use super::changelog::print_section_changelog;
use super::files::print_files_section;
use super::preamble::print_preamble_content;
use super::scriptlet::{print_file_trigger, print_scriptlet, print_trigger};
use super::text::{print_body_literal_escaped, print_text};
use super::util::print_subpkg;
use super::{Printer, TokenKind};

pub(crate) fn print_section<T>(p: &mut Printer<'_>, section: &Section<T>) {
    match section {
        Section::Description { subpkg, body, .. } => print_description(p, subpkg.as_ref(), body),
        Section::Package {
            name_arg, content, ..
        } => print_package(p, name_arg, content),
        Section::BuildScript { kind, body, .. } => print_build_script(p, *kind, body),
        Section::Files {
            subpkg,
            file_lists,
            content,
            ..
        } => print_files_section(p, subpkg.as_ref(), file_lists, content),
        Section::Scriptlet(s) => print_scriptlet(p, s),
        Section::Trigger(t) => print_trigger(p, t),
        Section::FileTrigger(ft) => print_file_trigger(p, ft),
        Section::Verify { subpkg, body, .. } => print_verify_section(p, subpkg.as_ref(), body),
        Section::Changelog { entries, .. } => print_section_changelog(p, entries),
        Section::SourceList { entries, .. } => print_list_section(p, "%sourcelist", entries),
        Section::PatchList { entries, .. } => print_list_section(p, "%patchlist", entries),
        Section::Sepolicy { subpkg, body, .. } => print_sepolicy(p, subpkg.as_ref(), body),
    }
}

// ---------------------------------------------------------------------
// %description
// ---------------------------------------------------------------------

fn print_description(p: &mut Printer<'_>, subpkg: Option<&SubpkgRef>, body: &TextBody) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, "%description");
    print_subpkg(p, subpkg);
    p.newline();
    for line in &body.lines {
        p.write_indent();
        print_body_literal_escaped(p, line, TokenKind::TextBody);
        p.newline();
    }
}

// ---------------------------------------------------------------------
// %package
// ---------------------------------------------------------------------

fn print_package<T>(p: &mut Printer<'_>, name: &PackageName, content: &[PreambleContent<T>]) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, "%package");
    match name {
        PackageName::Absolute(t) => {
            p.raw(" -n ");
            print_text(p, t);
        }
        PackageName::Relative(t) => {
            p.raw_char(' ');
            print_text(p, t);
        }
    }
    p.newline();
    for item in content {
        print_preamble_content(p, item);
    }
}

// ---------------------------------------------------------------------
// Build-scripts
// ---------------------------------------------------------------------

fn print_build_script(p: &mut Printer<'_>, kind: BuildScriptKind, body: &ShellBody) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, build_script_keyword(kind));
    p.newline();
    print_shell_body(p, body);
}

fn build_script_keyword(k: BuildScriptKind) -> &'static str {
    match k {
        BuildScriptKind::Prep => "%prep",
        BuildScriptKind::Conf => "%conf",
        BuildScriptKind::Build => "%build",
        BuildScriptKind::Install => "%install",
        BuildScriptKind::Check => "%check",
        BuildScriptKind::Clean => "%clean",
        BuildScriptKind::GenerateBuildRequires => "%generate_buildrequires",
    }
}

// ---------------------------------------------------------------------
// %verify / %sepolicy
// ---------------------------------------------------------------------

fn print_verify_section(p: &mut Printer<'_>, subpkg: Option<&SubpkgRef>, body: &ShellBody) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, "%verify");
    print_subpkg(p, subpkg);
    p.newline();
    print_shell_body(p, body);
}

fn print_sepolicy(p: &mut Printer<'_>, subpkg: Option<&SubpkgRef>, body: &ShellBody) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, "%sepolicy");
    print_subpkg(p, subpkg);
    p.newline();
    print_shell_body(p, body);
}

// ---------------------------------------------------------------------
// %sourcelist / %patchlist
// ---------------------------------------------------------------------

fn print_list_section(p: &mut Printer<'_>, keyword: &str, entries: &[Text]) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, keyword);
    p.newline();
    for entry in entries {
        p.write_indent();
        print_text(p, entry);
        p.newline();
    }
}

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

fn print_shell_body(p: &mut Printer<'_>, body: &ShellBody) {
    for line in &body.lines {
        p.write_indent();
        print_body_literal_escaped(p, line, TokenKind::ShellBody);
        p.newline();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::PrinterConfig;

    fn render<T>(s: &Section<T>) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_section(&mut p, s);
        buf
    }

    #[test]
    fn description_main() {
        let s: Section<()> = Section::Description {
            subpkg: None,
            body: TextBody {
                lines: vec![Text::from("hi"), Text::from("there")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%description\nhi\nthere\n");
    }

    #[test]
    fn description_subpkg() {
        let s: Section<()> = Section::Description {
            subpkg: Some(SubpkgRef::Absolute(Text::from("libfoo"))),
            body: TextBody {
                lines: vec![Text::from("body")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%description -n libfoo\nbody\n");
    }

    #[test]
    fn prep_section() {
        let s: Section<()> = Section::BuildScript {
            kind: BuildScriptKind::Prep,
            body: ShellBody {
                lines: vec![Text::from("autosetup")],
            },
            data: (),
        };
        assert_eq!(render(&s), "%prep\nautosetup\n");
    }

    #[test]
    fn sourcelist() {
        let s: Section<()> = Section::SourceList {
            entries: vec![Text::from("a.tar.gz"), Text::from("b.tar.gz")],
            data: (),
        };
        assert_eq!(render(&s), "%sourcelist\na.tar.gz\nb.tar.gz\n");
    }
}
