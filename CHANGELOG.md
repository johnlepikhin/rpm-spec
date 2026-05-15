# Changelog

All notable changes to `rpm-spec` are documented here.

The format roughly follows [Keep a Changelog](https://keepachangelog.com/),
and this crate adheres to [Semantic Versioning](https://semver.org/) once
it reaches `0.1.0`.

## 0.3.1

### Fixed

- Multi-dep preamble lines (`BuildRequires: a, b, c`,
  `Requires: x, y >= 1.0`, and every other dep-bearing tag) now carry a
  distinct per-atom `Span` on each split `SpecItem::Preamble`. Previously
  every split item inherited the whole-line span, which broke
  source-byte slicing in hoist / dedup analyzers. Single-atom lines keep
  the whole-line span so autofixers can still remove the entire line via
  `body_span`. Affects: `Requires`, `BuildRequires`, `Provides`,
  `Conflicts`, `Obsoletes`, `Recommends`, `Suggests`, `Supplements`,
  `Enhances`, `BuildConflicts`, `OrderWithRequires`.

## 0.3.0

### Added

- Pretty-printer now emits a category-aware token stream consumable by
  syntax highlighters, ANSI colorizers, and IDE tooling:
  - `printer::TokenKind` — `#[non_exhaustive]` enum with 18 variants
    (preamble tags, section keywords, conditional keywords, macro
    flavours, `%if` operands/operators, comments, body text, modifier
    flags). `Plain` is the default for neutral whitespace and
    punctuation.
  - `printer::PrintWriter` — single-method trait
    `emit(&mut self, kind, text)`. Documented as infallible / in-memory
    only; ANSI-style example included.
  - `impl PrintWriter for String` — preserves byte-identical output for
    existing `print` / `print_with` callers.
  - `printer::print_to(spec, cfg, &mut dyn PrintWriter)` — entry point
    for category-aware sinks.
- File directives (`%doc`, `%license`, `%attr`, `%defattr`, `%config`,
  `%verify`, `%dir`, `%ghost`, `%lang`, `%caps`, `%artifact`,
  `%missingok`) classified as `MacroRef`; `%files` and scriptlet /
  trigger / file-trigger keywords as `SectionKeyword`.
- `text::print_body_literal_escaped` helper centralises the `%` → `%%`
  body-line escape used by changelog, description, and shell-body
  rendering.
- New tests: `classifies_specific_token_kinds`,
  `statement_emits_atomic_macro_ref_chunk`,
  `consecutive_sections_separated_by_single_blank_line`, plus
  `classified_writer_concatenates_to_plain_print` /
  `classified_writer_emits_at_least_one_semantic_token` round-trip
  guards.

### Changed

- Crate-level doc (`src/lib.rs`) rewritten — removed pre-alpha / stub
  language and added a runnable quick-start example.

## 0.2.0

### Added

- Structured parser for `%if` / `%elif` expressions (`ast::expr::ExprAst`
  + `BinOp`). Recognises integer/string/macro/identifier primaries,
  parenthesised sub-expressions, unary `!`, and binary `||`, `&&`, `==`,
  `!=`, `<`, `>`, `<=`, `>=` with conventional precedence. Every node
  carries a span when produced by the span-aware parser.
- New `CondExpr::Parsed(Box<ExprAst<T>>)` variant — populated when the
  full expression head fits the modelled grammar. Arithmetic
  (`+`, `-`, `*`, `/`) and other unmodelled constructs continue to land
  in `CondExpr::Raw` and round-trip bit-identically.
- Recursion-depth guard (`MAX_DEPTH = 128`) protects the expression
  parser from adversarial input like `!!…!!1` or `(((…)))`.
- Three roundtrip tests (`parsed_expressions_roundtrip`,
  `parsed_elif_expression_roundtrips`, `unmodelled_expr_falls_back_to_raw`)
  alongside expanded expression-parser unit coverage.

### Changed

- **BREAKING:** `CondExpr` gained a type parameter (`CondExpr<T = ()>`)
  so that `Parsed` can carry per-node spans. The default `T = ()` keeps
  most usages compiling, but downstream code that names the type
  parameter explicitly, implements traits over the enum, or destructures
  the previously-monomorphic shape needs to be adjusted.
- `parser::expr::parse_expression` is `pub(crate)` — the structured
  parser is reachable only via the conditional path.

## 0.1.0

### Added

- Public `printer::FEDORA_PREAMBLE_VALUE_COLUMN` constant for the
  default preamble value alignment column (was a magic `16`).
- Stable diagnostic codes (`rpmspec/E####` / `rpmspec/W####`) attached
  to every parser warning/error via `Diagnostic::code`.
- `tracing` instrumentation under the optional `tracing` feature on
  `parse_section`, `parse_preamble_line`, `push_diagnostic` and the
  public entry points.
- Range warnings: file modes outside `0..=0o7777` (`rpmspec/W0018`)
  and changelog dates with implausible day/year
  (`rpmspec/W0025`).
- Integration test suite covering CRLF, deeply-nested rich deps,
  large-input stress, non-ASCII identifiers, mode-boundary warnings,
  and multi-line continuations.

### Changed

- **BREAKING** (pre-0.1, no published crate yet): `ParseError` reduced
  from 4 variants to 1 (`Io`). The removed variants (`Syntax`,
  `UnterminatedConditional`, `InvalidSection`) were never produced by
  the current entry points — every recoverable issue is reported via
  `Diagnostic`. The remaining `Io` variant is reserved for future
  `parse_reader` / `parse_file` entry points. Because the enum is
  `#[non_exhaustive]`, downstream code must already include a wildcard
  arm.
- Removed the `pretty` workspace dependency (it was declared but
  never used by the simple-string-builder printer).
- Reduced `N×clone` in `build_preamble_items` for multi-dep preamble
  lines (`Requires: foo bar baz`). The common single-dep case now
  performs **zero** clones.
- `printer/util.rs` consolidates the `print_subpkg` helper that was
  previously duplicated verbatim in three printer sub-modules.
- `W_MALFORMED_CHANGELOG_HEADER` (`rpmspec/W0023`) no longer fires on
  implausible day/year. Those now produce `W_IMPLAUSIBLE_CHANGELOG_DATE`
  (`rpmspec/W0025`). Downstream consumers matching on diagnostic codes
  must add the new code to their handler.
- `%attr` / `%defattr` numeric mode detection rejects tokens
  containing digits `8` or `9` up front (treated as user/group names),
  rather than falling through `from_str_radix` failure.

### Fixed

- Lossy `.map_err(|_| ...)` calls in `parser/changelog.rs` that
  collapsed `nom::Err::Failure` to `Error` and lost source spans.
- `parser/util.rs::logical_line` continuation-detection logic
  rewritten with a single bool flag for clarity.
- Stale design-narrative comment in
  `printer/macros.rs::print_body_with_continuations`.
