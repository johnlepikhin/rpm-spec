//! Edge-case coverage that single-purpose stage tests don't exercise:
//! CRLF line endings, deeply-nested rich deps, large inputs, non-ASCII
//! identifiers, file-mode boundary, multiple `\` continuations.

use rpm_spec::ast::{BoolDep, DepExpr, SpecItem, TagValue};
use rpm_spec::parser::parse_str;
use rpm_spec::printer::print;

#[test]
fn crlf_line_endings_parse_and_roundtrip() {
    let src = "Name: hello\r\nVersion: 1.0\r\nRelease: 1\r\nSummary: x\r\nLicense: MIT\r\n";
    let r = parse_str(src);
    let preambles: Vec<_> = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::Preamble(_)))
        .collect();
    assert_eq!(preambles.len(), 5);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);

    // Round-trip — printer emits LF; second parse must match.
    let printed = print(&r.spec);
    let r2 = parse_str(&printed);
    assert_eq!(r.spec, r2.spec);
}

#[test]
fn deeply_nested_rich_dep() {
    // Five levels of nesting: ((((A and B) or C) with D) without E).
    let src = "Requires: ((((a and b) or c) with d) without e)\n";
    let r = parse_str(src);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    let dep = match &r.spec.items[0] {
        SpecItem::Preamble(p) => match &p.value {
            TagValue::Dep(d) => d,
            _ => panic!(),
        },
        _ => panic!(),
    };

    // Walk all four BoolDep levels: Without → With → Or → And → atoms.
    let outer = match dep {
        DepExpr::Rich(b) => b.as_ref(),
        _ => panic!("expected Rich at level 1"),
    };
    let lvl_with = match outer {
        BoolDep::Without { left, .. } => match left.as_ref() {
            DepExpr::Rich(b) => b.as_ref(),
            _ => panic!("level 2: expected Rich under Without.left"),
        },
        _ => panic!("level 1: expected Without"),
    };
    let lvl_or = match lvl_with {
        BoolDep::With(operands) => match operands.first() {
            Some(DepExpr::Rich(b)) => b.as_ref(),
            _ => panic!("level 3: expected Rich as first With operand"),
        },
        _ => panic!("level 2: expected With"),
    };
    let lvl_and = match lvl_or {
        BoolDep::Or(operands) => match operands.first() {
            Some(DepExpr::Rich(b)) => b.as_ref(),
            _ => panic!("level 4: expected Rich as first Or operand"),
        },
        _ => panic!("level 3: expected Or"),
    };
    assert!(
        matches!(lvl_and, BoolDep::And(_)),
        "level 4: expected And, got {lvl_and:?}"
    );
}

#[test]
fn long_input_completes() {
    // 5 000 simple preamble lines + a description body.
    let mut src = String::with_capacity(80_000);
    src.push_str("Name: stress\nVersion: 1.0\nRelease: 1\nSummary: x\nLicense: MIT\n");
    for i in 0..5_000 {
        src.push_str(&format!("Requires: pkg{i}\n"));
    }
    src.push_str("%description\nbody\n");

    let start = std::time::Instant::now();
    let r = parse_str(&src);
    let elapsed = start.elapsed();

    // Guard against O(N^2) regressions: 5k lines must parse in well under 5s
    // even on slow CI runners. A single-digit-ms baseline is typical.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "parse took {elapsed:?}, suspect O(N^2) regression"
    );

    let req_count = r
        .spec
        .items
        .iter()
        .filter(|i| matches!(i, SpecItem::Preamble(p) if matches!(p.tag, rpm_spec::ast::Tag::Requires)))
        .count();
    assert_eq!(req_count, 5_000);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn non_ascii_in_summary_round_trips() {
    let src = "Name: hello\nVersion: 1.0\nRelease: 1\nSummary: Привет мир 你好\nLicense: MIT\n";
    let r = parse_str(src);
    let summary = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Preamble(p) if matches!(p.tag, rpm_spec::ast::Tag::Summary) => Some(p),
            _ => None,
        })
        .unwrap();
    match &summary.value {
        TagValue::Text(t) => assert_eq!(t.literal_str(), Some("Привет мир 你好")),
        _ => panic!(),
    }
    // Round-trip
    let printed = print(&r.spec);
    let r2 = parse_str(&printed);
    assert_eq!(r.spec, r2.spec);
}

#[test]
fn file_mode_boundary_warning() {
    // 0o7777 is the maximum valid mode — should NOT warn.
    let ok_src = "%files\n%attr(7777,root,root) /usr/bin/x\n";
    let r_ok = parse_str(ok_src);
    assert!(
        !r_ok
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("rpmspec/W0018")),
        "{:?}",
        r_ok.diagnostics
    );

    // 0o10000 is out of range — should warn.
    let bad_src = "%files\n%attr(10000,root,root) /usr/bin/x\n";
    let r_bad = parse_str(bad_src);
    assert!(
        r_bad
            .diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("rpmspec/W0018")),
        "{:?}",
        r_bad.diagnostics
    );
}

#[test]
fn implausible_changelog_year_warning() {
    let src = "%changelog\n* Wed May 14 1969 X <x@x.com> - 1.0-1\n- old\n";
    let r = parse_str(src);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("rpmspec/W0025")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn invalid_day_changelog_warning() {
    let src = "%changelog\n* Wed May 99 2025 X <x@x.com> - 1.0-1\n- bad day\n";
    let r = parse_str(src);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("rpmspec/W0025")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn long_define_continuation_chain() {
    // 30 segments joined by 29 `\` continuations.
    let mut body = String::new();
    body.push_str("%define long ");
    for i in 0..30 {
        body.push_str(&format!("part{i} "));
        if i < 29 {
            body.push_str("\\\n");
        }
    }
    body.push('\n');
    let r = parse_str(&body);
    assert_eq!(r.spec.items.len(), 1, "{:?}", r.spec.items);
    match &r.spec.items[0] {
        SpecItem::MacroDef(m) => {
            assert!(m.body.literal_str().unwrap_or("").contains("part29"));
        }
        _ => panic!(),
    }
}

#[test]
fn diagnostic_codes_are_populated_on_warnings() {
    // 0o10000 = 4096 decimal — exceeds max valid mode 0o7777.
    let src = "%files\n%attr(10000,root,root) /usr/bin/x\n";
    let r = parse_str(src);
    let codes: Vec<_> = r
        .diagnostics
        .iter()
        .filter_map(|d| d.code.as_deref())
        .collect();
    assert!(!codes.is_empty(), "no codes in {:?}", r.diagnostics);
    assert!(
        codes.iter().any(|c| c.starts_with("rpmspec/")),
        "{codes:?}"
    );
}
