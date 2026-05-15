//! Integration tests for stage 2 of the parser: preamble, dependencies,
//! `%description`, `%package`.

use rpm_spec::ast::{
    BoolDep, DepExpr, PackageName, PreambleContent, Section, SpecItem, Tag, TagQualifier,
    TagValue, VerOp,
};
use rpm_spec::parser::parse_str;

const CANONICAL: &str = "\
Name:           hello
Version:        1.0
Release:        1%{?dist}
Summary:        Greets the world
License:        MIT
URL:            https://example.org/hello
BuildArch:      noarch
Source0:        hello-%{version}.tar.gz
Patch0:         fix-something.patch

BuildRequires:  gcc make
BuildRequires:  pkgconfig(glib-2.0) >= 2.74
Requires:       glibc bash
Requires(post,postun): /bin/sh
Requires:       (foo and bar)
Requires:       /usr/bin/awk

%description
Hello greets the world.
It has two lines.

%package -n libhello
Summary:        Greeter library
License:        MIT
Requires:       glibc

%description -n libhello
The library half of hello.

%files
/usr/bin/hello

%files -n libhello
/usr/lib/libhello.so.*

%changelog
* Wed May 14 2026 Maintainer <m@example.org> - 1.0-1
- initial packaging
";

#[test]
fn canonical_spec_parses_preamble_structurally() {
    let r = parse_str(CANONICAL);

    // Every `Name:`/`Version:`/etc. line is a structural preamble item.
    let name_items: Vec<_> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Preamble(p) => Some(p),
            _ => None,
        })
        .collect();
    assert!(
        name_items.iter().any(|p| matches!(p.tag, Tag::Name)),
        "Name: tag not found"
    );
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::Version)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::Release)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::Summary)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::License)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::URL)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::BuildArch)));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::Source(Some(0)))));
    assert!(name_items.iter().any(|p| matches!(p.tag, Tag::Patch(Some(0)))));
}

#[test]
fn multi_dep_lines_expand_into_multiple_items() {
    let r = parse_str(CANONICAL);
    // `BuildRequires: gcc make` should produce two items.
    let br: Vec<_> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Preamble(p) if matches!(p.tag, Tag::BuildRequires) => Some(p),
            _ => None,
        })
        .collect();
    // gcc, make, pkgconfig(glib-2.0) — 3 items at top level.
    assert_eq!(br.len(), 3);

    // `Requires: glibc bash` adds two more Requires; `Requires(post,postun):`
    // adds one with qualifiers; `Requires: (foo and bar)` adds one rich;
    // `Requires: /usr/bin/awk` adds one file-dep — total 5.
    let req: Vec<_> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Preamble(p) if matches!(p.tag, Tag::Requires) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(req.len(), 5, "{req:?}");
}

#[test]
fn qualifier_list_preserved() {
    let r = parse_str(CANONICAL);
    let q = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p)
                if matches!(p.tag, Tag::Requires) && !p.qualifiers.is_empty() =>
            {
                Some(p)
            }
            _ => None,
        })
        .expect("Requires(post,postun) line not found");
    assert_eq!(
        q.qualifiers,
        vec![TagQualifier::Post, TagQualifier::Postun]
    );
}

#[test]
fn rich_dep_parsed() {
    let r = parse_str(CANONICAL);
    let rich = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p) => match &p.value {
                TagValue::Dep(DepExpr::Rich(b)) => Some(b.as_ref()),
                _ => None,
            },
            _ => None,
        })
        .expect("rich dep not found");
    assert!(matches!(rich, BoolDep::And(_)));
}

#[test]
fn pkgconfig_provider_no_arch() {
    let r = parse_str(CANONICAL);
    let pkgconf = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p) => match &p.value {
                TagValue::Dep(DepExpr::Atom(a))
                    if a.name.literal_str() == Some("pkgconfig(glib-2.0)") =>
                {
                    Some(a)
                }
                _ => None,
            },
            _ => None,
        })
        .expect("pkgconfig atom not found");
    assert!(pkgconf.arch.is_none());
    let c = pkgconf.constraint.as_ref().expect("constraint");
    assert_eq!(c.op, VerOp::Ge);
    assert_eq!(c.evr.version.literal_str(), Some("2.74"));
}

#[test]
fn file_dep_is_atom() {
    let r = parse_str(CANONICAL);
    let file_dep = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p) => match &p.value {
                TagValue::Dep(DepExpr::Atom(a))
                    if a.name.literal_str() == Some("/usr/bin/awk") =>
                {
                    Some(a)
                }
                _ => None,
            },
            _ => None,
        })
        .expect("file dep not found");
    assert!(file_dep.constraint.is_none());
}

#[test]
fn release_has_dist_macro() {
    let r = parse_str(CANONICAL);
    let release = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p) if matches!(p.tag, Tag::Release) => Some(p),
            _ => None,
        })
        .expect("Release line");
    match &release.value {
        TagValue::Text(t) => {
            assert!(
                t.segments
                    .iter()
                    .any(|s| matches!(s, rpm_spec::ast::TextSegment::Macro(m) if m.name == "dist"))
            );
        }
        _ => panic!(),
    }
}

#[test]
fn description_section_present() {
    let r = parse_str(CANONICAL);
    let descs: Vec<_> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Description { body, subpkg, .. } => Some((subpkg.clone(), body.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(descs.len(), 2, "expected main + libhello description");
    // Main description has no subpkg.
    assert!(descs.iter().any(|(s, _)| s.is_none()));
}

#[test]
fn package_subpackage_with_nested_preamble() {
    let r = parse_str(CANONICAL);
    let pkg = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Package { name_arg, content, .. } => Some((name_arg.clone(), content.clone())),
                _ => None,
            },
            _ => None,
        })
        .expect("%package -n libhello");
    match pkg.0 {
        PackageName::Absolute(t) => assert_eq!(t.literal_str(), Some("libhello")),
        _ => panic!("expected absolute name"),
    }
    let items: Vec<_> = pkg
        .1
        .iter()
        .filter_map(|c| match c {
            PreambleContent::Item(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(items.len(), 3, "Summary + License + Requires inside %package");
    assert!(items.iter().any(|p| matches!(p.tag, Tag::Summary)));
    assert!(items.iter().any(|p| matches!(p.tag, Tag::License)));
    assert!(items.iter().any(|p| matches!(p.tag, Tag::Requires)));
}

#[test]
fn no_deferred_diagnostics_after_stage3() {
    let r = parse_str(CANONICAL);
    let deferred: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not yet implemented"))
        .collect();
    assert!(deferred.is_empty(), "{deferred:?}");
}

#[test]
fn no_unrecognized_lines() {
    let r = parse_str(CANONICAL);
    let unrec: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not recognized"))
        .collect();
    assert!(unrec.is_empty(), "unexpected unrecognized lines: {unrec:?}");
}

#[test]
fn package_with_relative_name() {
    let r = parse_str("%package devel\nSummary: dev files\n");
    let s = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Package { name_arg, .. } => Some(name_arg.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("%package");
    match s {
        PackageName::Relative(t) => assert_eq!(t.literal_str(), Some("devel")),
        _ => panic!(),
    }
}
