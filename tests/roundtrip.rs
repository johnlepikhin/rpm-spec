//! Integration tests: `parse → print → parse → assert AST equality`.
//!
//! Equality is structural over `SpecFile<()>`. Multi-dep collapse is
//! the only intentional source of divergence — for that reason the
//! canonical spec is written without multi-dep lines, so item counts
//! match across the round-trip.

use rpm_spec::ast::SpecFile;
use rpm_spec::parser::parse_str;
use rpm_spec::printer::{PrintWriter, PrinterConfig, TokenKind, print, print_to, print_with};

const CANONICAL: &str = "\
Name:           hello
Version:        1.0
Release:        1%{?dist}
Summary:        Greets the world
License:        MIT
URL:            https://example.org/hello
BuildArch:      noarch
Source0:        hello-%{version}.tar.gz

BuildRequires:  gcc
BuildRequires:  make
Requires:       glibc

%description
The hello package greets the world.

%package -n libhello
Summary:        Greeter library
License:        MIT

%description -n libhello
Library half of hello.

%prep
%autosetup -p1

%build
%configure
%make_build

%install
%make_install

%check
make check

%files
%license LICENSE
%doc README.md
%attr(0755,root,root) /usr/bin/hello
%config(noreplace) /etc/hello.conf

%files -n libhello
%{_libdir}/libhello.so.*

%post -p /sbin/ldconfig

%post libhello -p /sbin/ldconfig

%triggerin -- foo
echo trigger fired

%filetriggerin -P 200 -- /usr/lib
do-something

%changelog
* Wed May 14 2025 Maintainer <m@example.org> - 1.0-1
- initial packaging
";

fn parse_to_unit(src: &str) -> SpecFile<()> {
    parse_str(src).spec
}

#[test]
fn canonical_roundtrip_default_config() {
    let ast1 = parse_to_unit(CANONICAL);
    let printed = print(&ast1);
    let ast2 = parse_to_unit(&printed);
    assert_eq!(
        ast1, ast2,
        "round-trip mismatch.\n\n=== PRINTED ===\n{printed}\n=== END ==="
    );
}

#[test]
fn canonical_roundtrip_indent_two_preserves_ast() {
    // With indent=2 the printed form has indented `%if` blocks; rpm
    // does not accept that, but our own parser does. Verify the AST is
    // unchanged after the round-trip.
    let ast1 = parse_to_unit(CANONICAL);
    let printed = print_with(&ast1, &PrinterConfig::new().with_indent(2));
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
}

const NESTED_COND: &str = "\
%if 1
%if 2
%define a 1
%endif
%endif
";

#[test]
fn nested_conditional_roundtrips() {
    let ast1 = parse_to_unit(NESTED_COND);
    let printed_flat = print(&ast1);
    let ast_flat = parse_to_unit(&printed_flat);
    assert_eq!(ast1, ast_flat, "flat: {printed_flat}");

    let printed_indented = print_with(&ast1, &PrinterConfig::new().with_indent(2));
    // Visual check: nested %if should be indented by 2 spaces.
    assert!(
        printed_indented.contains("\n  %if 2"),
        "indent missing in:\n{printed_indented}"
    );
    let ast_indented = parse_to_unit(&printed_indented);
    assert_eq!(ast1, ast_indented);
}

const FILES_WITH_COND: &str = "\
%description
hi

%files
/usr/bin/always
%if 0%{?fedora}
/usr/bin/fedora-only
%endif
";

#[test]
fn files_conditional_roundtrips_with_indent() {
    let ast1 = parse_to_unit(FILES_WITH_COND);
    let printed = print_with(&ast1, &PrinterConfig::new().with_indent(4));
    // The nested entry inside %if must be indented by 4 spaces.
    assert!(
        printed.contains("\n    /usr/bin/fedora-only"),
        "got:\n{printed}"
    );
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
}

#[test]
fn changelog_entry_roundtrips() {
    let src = "\
%changelog
* Mon Jan 01 2024 Alice <a@example.com> - 0.1-1
- first
- second
";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
    assert!(printed.contains("* Mon Jan 01 2024 Alice"));
}

#[test]
fn macro_definitions_roundtrip() {
    let src = "\
%global with_openssl 1
%define _hash bar
%bcond_with sqlite
%bcond_without gnutls
%include /etc/rpm/macros.fragment
%dnl a hidden note
";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
}

#[test]
fn parsed_expressions_roundtrip() {
    // Mix of `%if` expressions the AST should recognise structurally
    // (Integer, comparison, logical AND/OR, parens, string equality,
    // macro reference). The printer normalises whitespace around
    // operators — bit-identical roundtrip is not promised — but the
    // re-parsed AST must compare equal.
    let src = "\
%description
hi

%if 1
%define a 1
%endif

%if \"%{?_vendor}\" == \"suse\"
%define b 1
%endif

%if !1
%define c 1
%endif

%if 0%{?rhel} >= 8 && 0%{?rhel} < 10
%define d 1
%endif

%if (1 || 0) && %{?fedora}
%define e 1
%endif
";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2, "printed:\n{printed}");
}

#[test]
fn parsed_elif_expression_roundtrips() {
    // `%elif` must also reach the structured-parse path. A regression
    // that gates structured parsing on `%if` only would slip past
    // `parsed_expressions_roundtrip` above.
    let src = "\
%description
hi

%if 0
%define a 1
%elif %{?rhel} >= 8
%define b 1
%endif
";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2, "printed:\n{printed}");
}

#[test]
fn unmodelled_expr_falls_back_to_raw() {
    // Arithmetic (`+`) is outside the modelled expression grammar.
    // The parser must keep the line as `CondExpr::Raw` so the spec
    // file still round-trips bit-identically.
    let src = "\
%description
hi

%if 1 + 2 == 3
%define x 1
%endif
";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    // The raw expression must survive round-trip verbatim.
    assert!(
        printed.contains("%if 1 + 2 == 3"),
        "raw expression dropped:\n{printed}"
    );
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
}

#[test]
fn percent_in_literal_is_double_escaped() {
    // A literal `%` inside a preamble value must survive the round
    // trip — printer emits `%%`, parser decodes back to `%`.
    let src = "Name:           50%%off\n";
    let ast1 = parse_to_unit(src);
    let printed = print(&ast1);
    assert!(
        printed.contains("50%%off"),
        "expected `50%%off` in:\n{printed}"
    );
    let ast2 = parse_to_unit(&printed);
    assert_eq!(ast1, ast2);
}

/// `PrintWriter` consumers that classify tokens (e.g. ANSI highlighters
/// in downstream CLIs) must produce output that is byte-identical to
/// the default `String` writer once their categorization is dropped.
/// Any deviation would break round-trip — this guards against future
/// drift in how individual tokens are emitted.
#[derive(Default)]
struct CapturingWriter {
    /// All chunks emitted, with their classification.
    chunks: Vec<(TokenKind, String)>,
}

impl PrintWriter for CapturingWriter {
    fn emit(&mut self, kind: TokenKind, text: &str) {
        self.chunks.push((kind, text.to_string()));
    }
}

#[test]
fn classified_writer_concatenates_to_plain_print() {
    let ast = parse_to_unit(CANONICAL);
    let plain = print(&ast);
    let mut capturing = CapturingWriter::default();
    print_to(&ast, &PrinterConfig::default(), &mut capturing);
    let reconstructed: String = capturing.chunks.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(reconstructed, plain);
}

#[test]
fn classified_writer_emits_at_least_one_semantic_token() {
    // Sanity check that we're actually classifying — every non-trivial
    // spec should produce at least one non-Plain token (TagName from
    // the preamble alone is guaranteed).
    let ast = parse_to_unit(CANONICAL);
    let mut capturing = CapturingWriter::default();
    print_to(&ast, &PrinterConfig::default(), &mut capturing);
    let has_semantic = capturing
        .chunks
        .iter()
        .any(|(k, _)| !matches!(k, TokenKind::Plain));
    assert!(has_semantic, "expected at least one non-Plain token");
    let has_tag = capturing
        .chunks
        .iter()
        .any(|(k, _)| matches!(k, TokenKind::TagName));
    assert!(has_tag, "expected TagName tokens from preamble");
}

#[test]
fn classifies_specific_token_kinds() {
    // Hand-crafted spec exercising every interesting TokenKind we can
    // hit from the printer today. The `%if` block lives at the
    // *top level* (between the preamble and the first `%section`) so
    // the parser recognises it as a `Conditional` rather than swallowing
    // it as part of a section body — only then does the printer's
    // expression path (which emits String/Operator/Number chunks via
    // `print_expr_ast`) get exercised.
    let src = "\
Name:           foo
Version:        1.0

%if \"x\" == 0
%define a 1
%endif

%description
hi

%post
echo hi
";
    let ast = parse_to_unit(src);
    let mut w = CapturingWriter::default();
    print_to(&ast, &PrinterConfig::default(), &mut w);
    let has =
        |kind: TokenKind, text: &str| w.chunks.iter().any(|(k, s)| *k == kind && s.contains(text));
    assert!(
        has(TokenKind::TagName, "Name"),
        "expected (TagName, Name) in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::SectionKeyword, "%description"),
        "expected SectionKeyword for %description in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::SectionKeyword, "%post"),
        "expected SectionKeyword for %post in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::ConditionalKeyword, "%if"),
        "expected ConditionalKeyword for %if in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::ConditionalKeyword, "%endif"),
        "expected ConditionalKeyword for %endif in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::String, "\"x\""),
        "expected String literal token in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::Operator, "=="),
        "expected Operator for == in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::Number, "0"),
        "expected Number for 0 in {:?}",
        w.chunks
    );
    assert!(
        has(TokenKind::MacroDefKeyword, "%define"),
        "expected MacroDefKeyword for %define in {:?}",
        w.chunks
    );
}

#[test]
fn statement_emits_atomic_macro_ref_chunk() {
    // `%autosetup` (or similar bare macro statement at the top level)
    // must arrive as a single MacroRef chunk so highlighter consumers
    // can color the whole invocation uniformly.
    let src = "%autosetup\n";
    let ast = parse_to_unit(src);
    let mut w = CapturingWriter::default();
    print_to(&ast, &PrinterConfig::default(), &mut w);
    let atomic = w
        .chunks
        .iter()
        .find(|(k, s)| matches!(k, TokenKind::MacroRef) && s.starts_with("%autosetup"));
    assert!(
        atomic.is_some(),
        "expected one atomic MacroRef chunk starting with %autosetup, got: {:?}",
        w.chunks
    );
}

#[test]
fn consecutive_sections_separated_by_single_blank_line() {
    // Regression guard for `Printer::ends_with_blank_line` —
    // back-to-back sections must not collapse into one line or
    // gain extra blank lines.
    let src = "\
%description
hi

%prep
true

%build
true
";
    let ast = parse_to_unit(src);
    let printed = print(&ast);
    // Exactly one blank line between consecutive section headers.
    assert!(
        printed.contains("hi\n\n%prep"),
        "missing blank before %prep:\n{printed}"
    );
    assert!(
        printed.contains("true\n\n%build"),
        "missing blank before %build:\n{printed}"
    );
    // No triple-newline runs.
    assert!(
        !printed.contains("\n\n\n"),
        "unexpected triple newline:\n{printed}"
    );
}
