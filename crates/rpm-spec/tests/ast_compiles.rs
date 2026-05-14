//! Smoke test: construct a small SpecFile by hand and verify structural
//! equality. This catches the obvious "AST does not compile" bugs without
//! depending on the (yet unwritten) parser or printer.

use pretty_assertions::assert_eq;
use rpm_spec::ast::{
    AttrField, FileDirective, FileEntry, FilePath, FilesContent, PreambleContent, PreambleItem,
    Section, SpecFile, SpecItem, Tag, TagValue, Text, TextBody,
};

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

    let description = SpecItem::<()>::Section(Section::Description {
        subpkg: None,
        body:   TextBody { lines: vec![Text::from("Greets the world.")] },
        data:   (),
    });

    let files = SpecItem::<()>::Section(Section::Files {
        subpkg:     None,
        file_lists: vec![],
        content:    vec![FilesContent::Entry(FileEntry {
            directives: vec![FileDirective::Attr {
                mode:  AttrField::Numeric(0o755),
                user:  AttrField::Name(Text::from("root")),
                group: AttrField::Name(Text::from("root")),
            }],
            path:       Some(FilePath { path: Text::from("/usr/bin/hello") }),
            data:       (),
        })],
        data:       (),
    });

    let spec = SpecFile { items: vec![name, version, description, files], data: () };

    assert_eq!(spec.items.len(), 4);
    let _ = PreambleContent::<()>::Blank;
}
