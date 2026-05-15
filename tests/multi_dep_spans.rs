//! Per-atom span coverage for multi-dep preamble lines.
//!
//! `BuildRequires: a, b, c` parses into three [`SpecItem::Preamble`]
//! items. Each must carry a distinct, in-range span so downstream
//! linters that compare items via source-byte slicing can see the
//! individual atoms instead of the whole shared line.

use rpm_spec::ast::{Span, SpecItem, Tag};
use rpm_spec::parser::parse_str_with_spans;

fn dep_items(src: &str) -> Vec<(Tag, Span)> {
    let outcome = parse_str_with_spans(src);
    outcome
        .spec
        .items
        .into_iter()
        .filter_map(|item| match item {
            SpecItem::Preamble(p) => Some((p.tag, p.data)),
            _ => None,
        })
        .collect()
}

#[test]
fn multi_dep_atoms_have_disjoint_in_range_spans() {
    let src = "BuildRequires: alpha, beta >= 1.0, gamma\n";
    let items = dep_items(src);
    assert_eq!(items.len(), 3, "expected 3 split items, got {items:?}");

    // Parent line range (whole-line span uses 0..src.len()).
    let parent_end = src.len();

    let spans: Vec<Span> = items.iter().map(|(_, s)| *s).collect();
    for (i, sp) in spans.iter().enumerate() {
        assert!(sp.start_byte < sp.end_byte, "span {i} empty: {sp:?}");
        assert!(sp.end_byte <= parent_end, "span {i} overflow: {sp:?}");
    }
    // Pairwise disjoint: each next start is >= previous end.
    for w in spans.windows(2) {
        assert!(
            w[0].end_byte <= w[1].start_byte,
            "overlap: {:?} vs {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn multi_dep_span_slice_matches_atom_text() {
    let src = "BuildRequires: alpha, beta, gcc-c++\n";
    let items = dep_items(src);
    assert_eq!(items.len(), 3);

    // Verify each span's source slice contains the atom's name.
    let expected = ["alpha", "beta", "gcc-c++"];
    for ((_, sp), want) in items.iter().zip(expected.iter()) {
        let slice = &src[sp.start_byte..sp.end_byte];
        assert!(
            slice.trim().starts_with(want),
            "span slice `{slice}` does not start with `{want}` (span: {sp:?})"
        );
    }
}

#[test]
fn single_dep_keeps_whole_line_span() {
    // For one-atom lines the span must remain whole-line so that
    // autofixers (e.g. `useless-explicit-provides --fix`) can remove
    // the entire line by reading `body_span`.
    let src = "Provides: hello\n";
    let items = dep_items(src);
    assert_eq!(items.len(), 1);
    let (_, sp) = items[0];
    assert_eq!(sp.start_byte, 0, "single-atom span must start at line start");
    assert_eq!(
        sp.end_byte,
        src.len(),
        "single-atom span must cover the whole line"
    );
}

#[test]
fn multi_dep_first_atom_excludes_tag_prefix() {
    // The first atom's span must start AFTER the `Tag: ` prefix —
    // otherwise hoist-rule source-byte equality across branches would
    // still see different bytes (different tag prefixes are fine, but
    // the leading-prefix byte position would shift for `Provides:` vs
    // `Requires:`).
    let src = "BuildRequires: foo, bar\n";
    let items = dep_items(src);
    assert_eq!(items.len(), 2);
    let (_, first) = items[0];
    // `BuildRequires:` is 14 bytes + 1 space = 15.
    assert!(
        first.start_byte >= 15,
        "first atom span starts before/at tag prefix: {first:?}"
    );
    let slice = &src[first.start_byte..first.end_byte];
    assert!(slice.starts_with("foo"), "expected 'foo', got `{slice}`");
}
