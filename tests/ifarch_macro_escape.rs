//! Regression: `%%{X}` in `%ifarch` / `%if` arguments must survive a
//! round-trip through the pretty-printer unchanged.
//!
//! Background: in spec syntax `%%{X}` is the escaped form of the
//! literal text `%{X}` (rpm decodes `%%` → `%`). The conditional head
//! parser previously stored the raw lexeme — `%%{ix86}` — as a single
//! `TextSegment::Literal`. The pretty-printer's
//! `print_literal_escaped` then re-escaped each `%`, emitting
//! `%%%%{ix86}` on output. Successive `format --in-place` runs
//! cascaded the corruption (`%%%%%%%%{ix86}`, …).
//!
//! The fix is to route each head token through the full `Text`
//! grammar (`parse_body_as_text`) so the `%%` → `%` decoding happens
//! at parse time and the printer sees the canonical AST shape.

use rpm_spec::parser::parse_str_with_spans;
use rpm_spec::printer::{PrinterConfig, print_with};

fn pretty(src: &str) -> String {
    let outcome = parse_str_with_spans(src);
    print_with(&outcome.spec, &PrinterConfig::default())
}

#[test]
fn ifarch_macro_escape_is_idempotent() {
    let src = "%ifarch %%{ix86} x86_64\n%endif\n";
    let p1 = pretty(src);
    let p2 = pretty(&p1);
    assert_eq!(p1, p2, "pretty must be idempotent on %%{{X}} escapes");
    assert!(
        p1.contains("%%{ix86}"),
        "first pass keeps single escape: {p1}"
    );
    assert!(!p1.contains("%%%%{"), "no double escape: {p1}");
}

#[test]
fn ifarch_with_macro_alongside_plain_arch() {
    let src = "%ifarch aarch64 x86_64 %%{ix86}\n%endif\n";
    let p1 = pretty(src);
    let p2 = pretty(&p1);
    assert_eq!(p1, p2);
    assert!(!p1.contains("%%%%{"), "{p1}");
}

#[test]
fn if_with_macro_escape() {
    // If `%%{...}` also appears in `%if` expressions, verify too.
    // When the `%if` parser doesn't path through `ArchList` (the
    // structured expression grammar succeeds first) this test simply
    // documents current behaviour — idempotence is the load-bearing
    // invariant either way.
    let src = "%if %%{some_macro}\n%endif\n";
    let p1 = pretty(src);
    let p2 = pretty(&p1);
    assert_eq!(p1, p2);
}
