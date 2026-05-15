//! Smoke test: construct a small `SpecFile` by hand, exercise every major
//! AST type, and verify that `Send + Sync` are inherited for `T = ()`.

use pretty_assertions::assert_eq;
use rpm_spec::ast::{
    AttrField, AttrFields, BoolDep, BuildCondStyle, BuildCondition, ChangelogDate, ChangelogEntry,
    Comment, CommentStyle, CondBranch, CondExpr, CondKind, Conditional, DepAtom, DepExpr,
    FileDirective, FileEntry, FilePath, FilesContent, IncludeDirective, MacroDef, MacroDefKind,
    Month, PreambleContent, PreambleItem, Section, SpecFile, SpecItem, Tag, TagValue, Text,
    TextBody, Weekday,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn auto_traits() {
    assert_send_sync::<SpecFile<()>>();
    assert_send_sync::<SpecItem<()>>();
    assert_send_sync::<Section<()>>();
}

#[test]
fn build_minimal_spec() {
    let name = SpecItem::<()>::Preamble(PreambleItem {
        tag:        Tag::Name,
        qualifiers: vec![],
        lang:       None,
        value:      TagValue::Text(Text::from("hello")),
        data:       (),
    });

    let version = SpecItem::<()>::Preamble(PreambleItem {
        tag:        Tag::Version,
        qualifiers: vec![],
        lang:       None,
        value:      TagValue::Text(Text::from("1.0")),
        data:       (),
    });

    let description = SpecItem::<()>::section(Section::Description {
        subpkg: None,
        body:   TextBody { lines: vec![Text::from("Greets the world.")] },
        data:   (),
    });

    let files = SpecItem::<()>::section(Section::Files {
        subpkg:     None,
        file_lists: vec![],
        content:    vec![FilesContent::Entry(FileEntry {
            directives: vec![FileDirective::Attr(Box::new(AttrFields {
                mode:  AttrField::Numeric(0o755),
                user:  AttrField::Name(Text::from("root")),
                group: AttrField::Name(Text::from("root")),
            }))],
            path:       Some(FilePath { path: Text::from("/usr/bin/hello") }),
            data:       (),
        })],
        data:       (),
    });

    let spec = SpecFile { items: vec![name, version, description, files], data: () };

    assert_eq!(spec.items.len(), 4);
}

#[test]
fn build_complex_items() {
    let macro_def: SpecItem<()> = SpecItem::MacroDef(MacroDef {
        kind:     MacroDefKind::Global,
        name:     "foo".into(),
        opts:     None,
        body:     Text::from("bar"),
        eager:    false,
        global:   true,
        literal:  false,
        one_shot: false,
        data:     (),
    });

    let bcond: SpecItem<()> = SpecItem::BuildCondition(BuildCondition {
        style:   BuildCondStyle::Bcond,
        name:    "openssl".into(),
        default: Some(Text::from("1")),
        data:    (),
    });

    let include: SpecItem<()> = SpecItem::Include(IncludeDirective {
        path: Text::from("/etc/rpm/macros.fragment"),
        data: (),
    });

    let comment: SpecItem<()> = SpecItem::Comment(Comment {
        style: CommentStyle::Hash,
        text:  Text::from("workaround for bug #42"),
        data:  (),
    });

    let cond: SpecItem<()> = SpecItem::Conditional(Conditional {
        branches:  vec![CondBranch {
            kind: CondKind::IfArch,
            expr: CondExpr::ArchList(vec![Text::from("x86_64")]),
            body: vec![],
            data: (),
        }],
        otherwise: None,
        data:      (),
    });

    let _ = [macro_def, bcond, include, comment, cond];
}

#[test]
fn build_dependency_tree() {
    let atom = DepExpr::Atom(DepAtom {
        name:       Text::from("glibc"),
        arch:       None,
        constraint: None,
    });
    let rich = DepExpr::Rich(Box::new(BoolDep::If {
        cond:      Box::new(DepExpr::Atom(DepAtom {
            name:       Text::from("foo"),
            arch:       None,
            constraint: None,
        })),
        then:      Box::new(atom.clone()),
        otherwise: None,
    }));
    assert_ne!(atom, rich);
}

#[test]
fn build_changelog_entry() {
    let entry: ChangelogEntry<()> = ChangelogEntry {
        date:    ChangelogDate { weekday: Weekday::Wed, month: Month::May, day: 14, year: 2026 },
        author:  Text::from("Evgenii Lepikhin"),
        email:   Some(Text::from("johnlepikhin@gmail.com")),
        version: Some(Text::from("1.0-1")),
        body:    vec![Text::from("- initial release")],
        data:    (),
    };
    let _ = entry;
}

#[test]
fn preamble_content_blank_compiles() {
    let _ = PreambleContent::<()>::Blank;
}
