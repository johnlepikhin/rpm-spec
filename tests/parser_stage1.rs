//! Integration tests for stage 1 of the parser.

use rpm_spec::ast::{BuildCondStyle, MacroDefKind, SpecItem};
use rpm_spec::parser::{parse_str, parse_str_with_spans};

const SAMPLE: &str = "\
\u{feff}# A small sample exercising stage-1 parser features.
%global with_openssl 1
%define _hash %(printf '%%s' deadbeef)

%bcond_with sqlite
%bcond_without gnutls

%if 0%{?with_openssl}
%define crypto_lib openssl
%elif 0%{?with_gnutls}
%define crypto_lib gnutls
%else
%define crypto_lib none
%endif

%dnl this is a noisy reminder for packagers
%include /etc/rpm/macros.fragment

Name: hello
Version: 1.0
Release: 1%{?dist}
Summary: a greeter

%description
The hello package greets the world. Stage 1 does not yet parse this body.

%files
/usr/bin/hello
";

#[test]
fn sample_parses_with_expected_shape() {
    let r = parse_str(SAMPLE);

    // Macro statements / bconds / conditionals are recognized.
    let macro_defs = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::MacroDef(_)))
        .count();
    assert!(macro_defs >= 2, "expected at least 2 macrodefs, got {:?}", r.spec.items);

    let bconds = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::BuildCondition(_)))
        .count();
    assert_eq!(bconds, 2);

    let conds = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::Conditional(_)))
        .count();
    assert_eq!(conds, 1);

    let includes = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::Include(_)))
        .count();
    assert_eq!(includes, 1);

    // Stage 3 parses every section structurally — no deferred placeholders.
    let deferred = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not yet implemented"))
        .count();
    assert_eq!(deferred, 0, "no sections should defer at stage 3");

    let unrecognized = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not recognized"))
        .count();
    assert_eq!(unrecognized, 0, "preamble lines should parse cleanly: {:?}", r.diagnostics);
}

#[test]
fn span_aware_api_carries_positions() {
    let r = parse_str_with_spans(SAMPLE);
    // Find the first macro definition and check its span is non-empty and
    // covers its source line.
    let m = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::MacroDef(m) => Some(m),
            _ => None,
        })
        .expect("at least one MacroDef");
    assert!(m.data.end_byte > m.data.start_byte);
    assert!(m.data.start_line >= 1);
    assert!(m.data.end_line >= m.data.start_line);
}

#[test]
fn build_condition_with_default() {
    let r = parse_str("%bcond openssl 1\n");
    assert_eq!(r.spec.items.len(), 1);
    match &r.spec.items[0] {
        SpecItem::BuildCondition(b) => {
            assert_eq!(b.style, BuildCondStyle::Bcond);
            assert_eq!(b.name, "openssl");
            assert!(b.default.is_some());
        }
        other => panic!("expected BuildCondition, got {other:?}"),
    }
}

#[test]
fn nested_macros_recurse() {
    let r = parse_str_with_spans("%define url https://%{name}-%{version}.example/\n");
    match &r.spec.items[0] {
        SpecItem::MacroDef(m) => {
            assert_eq!(m.kind, MacroDefKind::Define);
            // body should contain at least one macro segment.
            let macros = m
                .body
                .segments
                .iter()
                .filter(|s| matches!(s, rpm_spec::ast::TextSegment::Macro(_)))
                .count();
            assert!(macros >= 2, "expected nested macros in body, got {:?}", m.body);
        }
        _ => panic!(),
    }
}
