//! `%files` body rendering.

use crate::ast::{
    AttrField, AttrFields, ConfigFlag, DefattrFields, FileDirective, FileEntry, FilePath,
    FilesContent, SubpkgRef, Text, VerifyCheck,
};

use super::cond::print_conditional;
use super::macros::print_comment;
use super::text::print_text;
use super::util::print_subpkg;
use super::{Printer, TokenKind};

/// Render a `Section::Files` body. The header itself is emitted by
/// `section.rs::print_section`.
pub(crate) fn print_files_section<T>(
    p: &mut Printer<'_>,
    subpkg: Option<&SubpkgRef>,
    file_lists: &[Text],
    content: &[FilesContent<T>],
) {
    p.write_indent();
    p.emit(TokenKind::SectionKeyword, "%files");
    print_subpkg(p, subpkg);
    for fl in file_lists {
        p.raw(" -f ");
        print_text(p, fl);
    }
    p.newline();
    for item in content {
        print_files_content(p, item);
    }
}

pub(crate) fn print_files_content<T>(p: &mut Printer<'_>, c: &FilesContent<T>) {
    match c {
        FilesContent::Entry(e) => print_file_entry(p, e),
        FilesContent::Conditional(cond) => {
            print_conditional(p, cond, |p, body| print_files_content(p, body))
        }
        FilesContent::Comment(cm) => print_comment(p, cm),
        FilesContent::Blank => p.newline(),
    }
}

fn print_file_entry<T>(p: &mut Printer<'_>, e: &FileEntry<T>) {
    p.write_indent();
    for (i, d) in e.directives.iter().enumerate() {
        if i > 0 {
            p.raw_char(' ');
        }
        print_directive(p, d);
    }
    if let Some(path) = &e.path {
        if !e.directives.is_empty() {
            p.raw_char(' ');
        }
        print_file_path(p, path);
    }
    p.newline();
}

fn print_directive(p: &mut Printer<'_>, d: &FileDirective) {
    match d {
        FileDirective::Defattr(f) => print_defattr(p, f),
        FileDirective::Attr(f) => print_attr(p, f),
        FileDirective::Dir => p.emit(TokenKind::MacroRef, "%dir"),
        FileDirective::Doc => p.emit(TokenKind::MacroRef, "%doc"),
        FileDirective::License => p.emit(TokenKind::MacroRef, "%license"),
        FileDirective::Config(flags) => print_config(p, flags),
        FileDirective::Ghost => p.emit(TokenKind::MacroRef, "%ghost"),
        FileDirective::Verify { negate, checks } => print_verify(p, *negate, checks),
        FileDirective::Lang(loc) => {
            p.emit(TokenKind::MacroRef, "%lang");
            p.raw("(");
            print_text(p, loc);
            p.raw_char(')');
        }
        FileDirective::Caps(spec) => {
            p.emit(TokenKind::MacroRef, "%caps");
            p.raw("(");
            print_text(p, spec);
            p.raw_char(')');
        }
        FileDirective::Artifact => p.emit(TokenKind::MacroRef, "%artifact"),
        FileDirective::MissingOk => p.emit(TokenKind::MacroRef, "%missingok"),
    }
}

fn print_defattr(p: &mut Printer<'_>, f: &DefattrFields) {
    p.emit(TokenKind::MacroRef, "%defattr");
    p.raw("(");
    print_attr_field(p, &f.fmode);
    p.raw_char(',');
    print_attr_field(p, &f.user);
    p.raw_char(',');
    print_attr_field(p, &f.group);
    if let Some(dmode) = &f.dmode {
        p.raw_char(',');
        print_attr_field(p, dmode);
    }
    p.raw_char(')');
}

fn print_attr(p: &mut Printer<'_>, f: &AttrFields) {
    p.emit(TokenKind::MacroRef, "%attr");
    p.raw("(");
    print_attr_field(p, &f.mode);
    p.raw_char(',');
    print_attr_field(p, &f.user);
    p.raw_char(',');
    print_attr_field(p, &f.group);
    p.raw_char(')');
}

fn print_attr_field(p: &mut Printer<'_>, a: &AttrField) {
    match a {
        AttrField::Default => p.raw_char('-'),
        AttrField::Numeric(n) => p.raw(&format!("{n:04o}")),
        AttrField::Name(t) => print_text(p, t),
    }
}

fn print_config(p: &mut Printer<'_>, flags: &[ConfigFlag]) {
    if flags.is_empty() {
        p.emit(TokenKind::MacroRef, "%config");
        return;
    }
    p.emit(TokenKind::MacroRef, "%config");
    p.raw("(");
    for (i, f) in flags.iter().enumerate() {
        if i > 0 {
            p.raw_char(',');
        }
        p.raw(config_flag_name(*f));
    }
    p.raw_char(')');
}

fn config_flag_name(f: ConfigFlag) -> &'static str {
    match f {
        ConfigFlag::NoReplace => "noreplace",
        ConfigFlag::MissingOk => "missingok",
    }
}

fn print_verify(p: &mut Printer<'_>, negate: bool, checks: &[VerifyCheck]) {
    p.emit(TokenKind::MacroRef, "%verify");
    p.raw("(");
    let mut first = true;
    if negate {
        p.raw("not");
        first = false;
    }
    for c in checks {
        if !first {
            p.raw_char(' ');
        }
        first = false;
        p.raw(verify_check_name(*c));
    }
    p.raw_char(')');
}

fn verify_check_name(c: VerifyCheck) -> &'static str {
    match c {
        VerifyCheck::Md5 => "md5",
        VerifyCheck::FileDigest => "filedigest",
        VerifyCheck::Size => "size",
        VerifyCheck::Link => "link",
        VerifyCheck::User => "user",
        VerifyCheck::Group => "group",
        VerifyCheck::Mtime => "mtime",
        VerifyCheck::Mode => "mode",
        VerifyCheck::Rdev => "rdev",
        VerifyCheck::Caps => "caps",
    }
}

fn print_file_path(p: &mut Printer<'_>, fp: &FilePath) {
    print_text(p, &fp.path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::PrinterConfig;

    fn render<T>(
        subpkg: Option<&SubpkgRef>,
        file_lists: &[Text],
        content: &[FilesContent<T>],
    ) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_files_section(&mut p, subpkg, file_lists, content);
        buf
    }

    fn entry(directives: Vec<FileDirective>, path: Option<FilePath>) -> FilesContent<()> {
        FilesContent::Entry(FileEntry {
            directives,
            path,
            data: (),
        })
    }

    #[test]
    fn empty_files_header() {
        let out: String = render::<()>(None, &[], &[]);
        assert_eq!(out, "%files\n");
    }

    #[test]
    fn header_with_subpkg_and_filelist() {
        let out: String = render::<()>(
            Some(&SubpkgRef::Absolute(Text::from("libfoo"))),
            &[Text::from("files.list")],
            &[],
        );
        assert_eq!(out, "%files -n libfoo -f files.list\n");
    }

    #[test]
    fn doc_directive_with_path() {
        let out: String = render(
            None,
            &[],
            &[entry(
                vec![FileDirective::Doc],
                Some(FilePath {
                    path: Text::from("README.md"),
                }),
            )],
        );
        assert_eq!(out, "%files\n%doc README.md\n");
    }

    #[test]
    fn attr_directive() {
        let af = FileDirective::Attr(Box::new(AttrFields {
            mode: AttrField::Numeric(0o755),
            user: AttrField::Name(Text::from("root")),
            group: AttrField::Name(Text::from("root")),
        }));
        let out: String = render(
            None,
            &[],
            &[entry(
                vec![af],
                Some(FilePath {
                    path: Text::from("/usr/bin/hello"),
                }),
            )],
        );
        assert_eq!(out, "%files\n%attr(0755,root,root) /usr/bin/hello\n");
    }

    #[test]
    fn defattr_dash() {
        let df = FileDirective::Defattr(Box::new(DefattrFields {
            fmode: AttrField::Default,
            user: AttrField::Name(Text::from("root")),
            group: AttrField::Name(Text::from("root")),
            dmode: Some(AttrField::Default),
        }));
        let out: String = render(None, &[], &[entry(vec![df], None)]);
        assert_eq!(out, "%files\n%defattr(-,root,root,-)\n");
    }

    #[test]
    fn config_noreplace() {
        let out: String = render(
            None,
            &[],
            &[entry(
                vec![FileDirective::Config(vec![ConfigFlag::NoReplace])],
                Some(FilePath {
                    path: Text::from("/etc/foo.conf"),
                }),
            )],
        );
        assert_eq!(out, "%files\n%config(noreplace) /etc/foo.conf\n");
    }

    #[test]
    fn verify_with_not() {
        let out: String = render(
            None,
            &[],
            &[entry(
                vec![FileDirective::Verify {
                    negate: true,
                    checks: vec![VerifyCheck::Md5, VerifyCheck::Size],
                }],
                Some(FilePath {
                    path: Text::from("/usr/bin/foo"),
                }),
            )],
        );
        assert_eq!(out, "%files\n%verify(not md5 size) /usr/bin/foo\n");
    }

    #[test]
    fn multiple_directives() {
        let attr = FileDirective::Attr(Box::new(AttrFields {
            mode: AttrField::Numeric(0o644),
            user: AttrField::Name(Text::from("root")),
            group: AttrField::Name(Text::from("root")),
        }));
        let conf = FileDirective::Config(vec![ConfigFlag::NoReplace]);
        let out: String = render(
            None,
            &[],
            &[entry(
                vec![attr, conf],
                Some(FilePath {
                    path: Text::from("/etc/foo.conf"),
                }),
            )],
        );
        assert_eq!(
            out,
            "%files\n%attr(0644,root,root) %config(noreplace) /etc/foo.conf\n"
        );
    }
}
