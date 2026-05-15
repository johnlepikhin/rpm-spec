# Changelog

All notable changes to `rpm-spec` are documented here.

The format roughly follows [Keep a Changelog](https://keepachangelog.com/),
and this crate adheres to [Semantic Versioning](https://semver.org/) once
it reaches `0.1.0`.

## Unreleased

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
