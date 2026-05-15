# rpm-spec

A parser and pretty-printer for RPM `.spec` files, written in Rust.

The crate exposes a distribution-independent abstract syntax tree, a recovery-oriented parser that never panics on real
input, and a configurable pretty-printer that re-emits a normalised but structurally equivalent source. It is intended
as a small, auditable building block for higher-level tooling — formatters, linters, static analyzers, and packaging
dashboards.

The parser does **not** expand macros. Macro references (`%name`, `%{name}`, `%[expr]`, `%(shell)`) are preserved as AST
nodes with their original spelling, so downstream tools can inspect or rewrite the source without losing structural
information. Distribution-specific macro registries are deliberately out of scope and belong in a separate validator
layer on top of this crate.

## Synopsis

```rust
use rpm_spec::parser::parse_str;
use rpm_spec::printer::{print, print_with, PrinterConfig};

fn main() {
    let source = "\
Name:           hello\n\
Version:        1.0\n\
Release:        1%{?dist}\n\
Summary:        Greeter\n\
License:        MIT\n\
\n\
%description\n\
Greets the world.\n\
\n\
%files\n\
/usr/bin/hello\n\
";

    let result = parse_str(source);

    for d in &result.diagnostics {
        eprintln!("{:?} {:?} {}", d.severity, d.code, d.message);
    }

    // Re-emit with the default (Fedora-style) layout.
    let rendered = print(&result.spec);
    assert!(rendered.contains("Name:"));

    // Or with explicit configuration.
    let cfg = PrinterConfig::new().with_indent(2);
    let _ = print_with(&result.spec, &cfg);
}
```

## Installation

The crate has not been published yet. While the API is in flux it is intended to be consumed as a git or path
dependency:

```toml
[dependencies]
rpm-spec = { git = "https://github.com/johnlepikhin/rpm-spec" }
```

`rpm-spec` requires Rust edition 2024 and a `rustc` recent enough to build it. The crate is `#![forbid(unsafe_code)]`
and pulls in `nom` and `nom_locate` only when the `parser` feature is enabled.

## Cargo features

The default feature set is `["parser", "printer"]`. Every feature is additive.

| Feature   | Default | Effect                                                                                                       |
| --------- | ------- | ------------------------------------------------------------------------------------------------------------ |
| `parser`  | yes     | Compiles the `parser` module and the `parse_str` / `parse_str_with_spans` entry points. Pulls in `nom`.      |
| `printer` | yes     | Compiles the `printer` module (`print`, `print_with`, `PrinterConfig`). No extra dependencies.               |
| `serde`   | no      | Derives `Serialize` / `Deserialize` on the AST, diagnostics and configuration types.                         |
| `tracing` | no      | Adds `#[tracing::instrument]` on hot-path entry points (`parse_str`, `parse_section`, `push_diagnostic`, …). |

To consume only the AST without either parser or printer:

```toml
rpm-spec = { git = "...", default-features = false }
```

## Description

### Module map

The crate is laid out so that the AST is usable on its own; the parser and printer plug into it from the side.

* `ast` — AST root and all node types. The root is `SpecFile<T>`; user-data parameter `T` defaults to `()`.
* `parse_result` — `ParseResult<T>` and `Diagnostic` returned by the parser, plus the `codes` module of stable
  identifiers.
* `parser` (feature `parser`) — `&str → ParseResult`.
* `printer` (feature `printer`) — `&SpecFile<T> → String`.
* `error` — fatal error types. Currently unused by `parse_str` / `parse_str_with_spans`; reserved for future
  `parse_reader` / `parse_file` entry points.

### AST shape

The AST is generic over a per-node user-data parameter `T`. The default is `()`, which produces a compact tree suitable
for printers and analyzers that do not care about source locations. Calling `parse_str_with_spans` populates `T` with
`ast::Span`, a byte-offset plus 1-based line and column at both ends. Validators that need to thread their own state
(resolved macro values, validator diagnostic ids, …) can choose a richer `T` and map between representations.

`SpecItem<T>` is the top-level enumeration: preamble lines (`Name:`, `Version:`, `Requires:`), sections (`%description`,
`%files`, `%prep`, …), conditional blocks (`%if`, `%ifarch`), macro definitions, `%bcond*` toggles, comments and blank
lines. Dependency expressions inside `Requires:` / `BuildRequires:` / `Provides:` / `Conflicts:` are decoded into a
typed `DepExpr` that supports classic atoms and RPM 4.13+ rich/boolean dependencies (`and`, `or`, `with`, `without`,
`if` / `unless` with optional `else`).

Several types are documented as **permissive**: implausible values are accepted by the parser, stored verbatim and
reported through a diagnostic, rather than rejected. This applies to `AttrField` (file modes outside `0..=0o7777`),
`ChangelogDate` (day outside `1..=31`, year outside `1970..=2200`) and similar positions.

### Parser

```rust
use rpm_spec::parser::{parse_str, parse_str_with_spans};

let r       = parse_str(source);            // ParseResult<()>
let r_spans = parse_str_with_spans(source); // ParseResult<Span>
```

The parser is recovery-oriented. Both entry points always return a (possibly partial) `SpecFile` and a
`Vec<Diagnostic>`; they do not return `Result`. Recoverable issues — unrecognised lines, malformed dependency
expressions, implausible changelog dates — are surfaced as `Diagnostic` entries and the parser resynchronises at the
next safe point. A fatal `ParseError` is reserved for I/O failures that the current string-based entry points cannot
produce.

The parser handles CRLF line endings, leading UTF-8 BOMs and multi-line `\`-continuations. Input is expected to be
valid UTF-8; legacy Windows-1251 spec files must be transcoded by the caller before parsing.

### Diagnostics

Every diagnostic carries:

* a `severity` (`Warning` or `Error`),
* an optional `span` pointing into the original source,
* a human-readable `message`,
* zero or more free-form `notes`, and
* an optional stable `code` from the `rpm_spec::parse_result::codes` module.

Diagnostic codes use the `rpmspec/E####` namespace for errors and `rpmspec/W####` for warnings. Codes are stable across
patch releases: new codes may be added, existing codes are never renumbered or repurposed. Consumers that want to
filter or suppress specific findings should match on `Diagnostic.code` rather than on substrings of `message`.

Selected codes:

| Code            | Meaning                                                                                  |
| --------------- | ---------------------------------------------------------------------------------------- |
| `rpmspec/E0001` | Parser made no progress at a position (guard against infinite loops in malformed input). |
| `rpmspec/E0002` | `%if` / `%ifarch` / `%ifos` block opened without a matching `%endif`.                    |
| `rpmspec/E0003` | Rich dependency mixes operators on the same level without explicit nesting.              |
| `rpmspec/W0001` | Stray `%` in text that did not form a valid macro reference.                             |
| `rpmspec/W0018` | Numeric file mode in `%attr` / `%defattr` exceeds `0o7777`.                              |
| `rpmspec/W0023` | A `%changelog` entry header was structurally unparseable.                                |
| `rpmspec/W0025` | A `%changelog` entry header parsed but the date is implausible.                          |

The full table lives in `parse_result::codes` with one constant per code.

### Pretty-printer

```rust
use rpm_spec::printer::{print, print_with, PrinterConfig, FEDORA_PREAMBLE_VALUE_COLUMN};

let default      = print(&spec);
let indented     = print_with(&spec, &PrinterConfig::new().with_indent(2));
let no_alignment = print_with(&spec, &PrinterConfig::new().with_preamble_value_column(None));
```

`PrinterConfig` carries two knobs:

| Field                   | Default                              | Effect                                                                        |
| ----------------------- | ------------------------------------ | ----------------------------------------------------------------------------- |
| `indent`                | `0`                                  | Spaces added per nesting level inside `%if` blocks.                           |
| `preamble_value_column` | `Some(FEDORA_PREAMBLE_VALUE_COLUMN)` | Column at which `Tag:` values are aligned. `None` always uses a single space. |

`FEDORA_PREAMBLE_VALUE_COLUMN` (currently 16) matches Fedora packaging conventions; if a tag's `Tag(qualifier):` prefix
already exceeds the configured column, a single space is used instead so values never overlap their headers.

The printer is a plain `&mut String` writer; it does not pull in a layout-engine dependency. Round-tripping
`parse → print → parse` preserves the AST modulo intentional normalisation (e.g. multi-dep `Requires: a b c` lines are
collapsed back from `N` AST items into a single source line).

### Error type

`error::ParseError` is `#[non_exhaustive]` and currently contains a single variant:

```rust
pub enum ParseError {
    Io { message: String },
}
```

The variant is reserved for future `parse_reader` / `parse_file` entry points; the existing string-based functions
never produce it. Downstream code must include a wildcard arm because the enum may grow without a major version bump
while it is marked non-exhaustive.

### Invariants worth knowing

* **Macro names are verbatim.** `MacroRef::name`, `MacroDef::name`, `BuildCondition::name`, and the `Other` variants of
  `Tag`, `TagQualifier` and `BuiltinMacro` preserve the exact text from the source — case is **not** normalised. This
  is what lets downstream validators match names against distribution-specific registries.
* **Span invariant.** `Span::start_byte <= end_byte`. `Span::new` and `Span::from_bytes` assert this in debug builds.
* **No `unsafe`.** The crate is `#![forbid(unsafe_code)]`.
* **Single-threaded parser state.** `ParserState` holds an `Rc<RefCell<...>>` and is neither `Send` nor `Sync`. Each
  call to `parse_str` allocates its own state, so concurrent parsing of independent inputs is simply a matter of
  driving one thread per spec.

## Building and testing

The crate compiles cleanly under default features, `--no-default-features`, and `--all-features`:

```sh
cargo build
cargo build --all-features
cargo check --no-default-features
cargo check --no-default-features --features serde

cargo test   --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All tests are offline. Integration coverage includes round-trip tests on canonical Fedora-shaped specs, CRLF inputs,
deeply nested rich dependencies, large-input stress, non-ASCII identifiers, and changelog / file-mode boundary
diagnostics.

## Status and stability

The crate is pre-`0.1`. The public surface is still subject to change — type renames, additional `Diagnostic` codes,
new variants on `#[non_exhaustive]` enums, and adjustments to printer layout heuristics are all on the table. Once a
crates.io release is cut, the usual semver guarantees will apply; until then, pin the dependency to a specific git
revision.

Diagnostic codes (`rpmspec/E####` / `rpmspec/W####`) are an exception: they are intended to be stable from the moment
they are introduced. New codes may be added, existing codes are not renumbered.

The parser targets the RPM `.spec` format as documented by the `rpm` project and as observed across Fedora, RHEL,
openSUSE and Mageia spec files in 2024–2025. Distribution-specific macro semantics (which `%foo` is defined by which
`/usr/lib/rpm/*-macros` file) are deliberately out of scope.

## License

Licensed under either of

* Apache License, Version 2.0
* MIT license

at your option.

## See also

* [`rpm`](https://github.com/rpm-software-management/rpm) — the upstream package manager and the canonical reference
  for the `.spec` format.
* [Fedora Packaging Guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/) and the openSUSE / Mageia
  packaging documentation — the practical conventions this crate aims to round-trip.

This crate is an independent implementation in Rust and is not affiliated with the `rpm` project or with any specific
distribution.
