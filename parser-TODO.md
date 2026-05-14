# Parser implementation roadmap

Pre-decided architectural points (recorded in chat):

- **Error model:** recovery + diagnostics. Each sub-parser, on failure,
  records a `Diagnostic`, synchronizes to the next `%name` header or `Tag:`
  line, and resumes. Returns partial AST plus all diagnostics.
- **Input type:** `nom_locate::LocatedSpan<&str>`.
- **Span shape:** `Span { start_byte, end_byte, start_line, start_column,
  end_line, end_column }` — byte offsets *and* line/column.
- **Shell sections** (`%prep`/`%build`/`%install`/scriptlets/triggers):
  parsed structurally. `ShellBody::lines: Vec<Text>` — every line is split
  into literal + macro segments.
- **Whitelisted section names:** `const &[&str]`, no `phf`.
- **`%if`/`%endif` inside shell bodies:** kept as raw text lines (per AST
  design), not as structural `Conditional` nodes.
- **Multi-dep `Requires:` lines** (`Requires: foo bar baz`) expand into N
  separate `PreambleItem`s sharing the same `Tag`.
- **Top-level macro statements** (`%dump`, `%trace`, bare `%lua{...}`, …)
  represented by a new variant `SpecItem::Statement(MacroRef)`.
- **Multi-line `%define` / `%global` body** stored with `\n` between
  source lines; the line-continuation backslash itself is **not** kept.
  Printer is responsible for re-emitting `\` when wrapping.
- **Macro body recursion:** `%(shell)`, `%[expr]`, `%{lua:...}`, and all
  builtin `%{shrink|expand|…:body}` forms parse their bodies as `Text`
  (recursive — nested macros become structured nodes, not flat strings).

## Stage 1 — Foundation *(in progress in this sitting)*

Modules to land:

- `ast::Span` — extend to 6 fields (`start_byte/end_byte/start_line/
  start_column/end_line/end_column`); update tests + lib doc.
- `Cargo.toml`: add `nom_locate` to workspace deps.
- `parser/state.rs` — `ParserState { config: Rc<ParserConfig>,
  diagnostics: Rc<RefCell<Vec<Diagnostic>>> }`. `ParserConfig` empty for
  now; placeholder for strict-mode toggles.
- `parser/input.rs` — type alias `pub type Input<'a> =
  nom_locate::LocatedSpan<&'a str>;` plus `span_between(from, to) -> Span`
  helper.
- `parser/util.rs` — BOM stripping, line-continuation collapser, generic
  whitespace eaters, line-tail scanner.
- `parser/text.rs` — `parse_text` (terminator-aware) and full macro-ref
  grammar:
  - `%foo` (Plain), `%{foo}` (Braced)
  - `%{?foo}`, `%{!?foo}` (conditional prefix)
  - `%{?foo:VALUE}`, `%{!?foo:VALUE}` (with_value)
  - `%{foo arg1 arg2}` (Parametric)
  - `%(shell command)` (Shell, balanced parens)
  - `%[expr]`, `%{expr:body}` (Expr)
  - `%{lua:body}` (Lua)
  - `%{shrink|expand|quote|gsub|sub|len|upper|lower|reverse|basename|
    dirname|suffix|exists|load|echo|warn|error|dnl|trace|dump:body}`
    (Builtin)
  - Positional refs: `%1`, `%*`, `%**`, `%#`, `%{-f}`, `%{-f*}`
  - `%%` → literal `%`
- `parser/macros.rs` — top-level macro statements:
  - `%define`/`%global`/`%undefine` (with `-e`, `-g`, `<l>`, `<o>`
    modifiers and `(opts)` parametric signature)
  - `%bcond`/`%bcond_with`/`%bcond_without`
  - `%include`
  - `%dnl` comments and `#` comments
- `parser/cond.rs` — top-level `Conditional<Span, SpecItem<Span>>`. Generic
  enough to be re-used inside preamble/files in stage 2/3 (with body =
  `PreambleContent`/`FilesContent`).
- `parser/mod.rs` — public entry points:
  - `pub fn parse_str(input: &str) -> ParseResult<()>` (spans discarded)
  - `pub fn parse_str_with_spans(input: &str) -> ParseResult<Span>`
  - top-level loop: blank → comment → macro stmt → conditional → section
    header (skip body w/ diagnostic) → unknown line (skip w/ diagnostic).

Out of scope for stage 1 (will emit "deferred" diagnostics for now):

- preamble lines (`Tag: value`)
- section bodies (everything between `%description`/`%prep`/etc. and the
  next section header is consumed but not parsed)
- file directives, changelog entries

Stage 1 acceptance:

- `cargo test --all-features` green; new tests cover every macro form,
  `%%` escape, conditional with elif/else, `%define` with multi-line body
  via `\`, BOM at start.
- A "deferred" diagnostic is emitted exactly once per unparsed section,
  not per line inside it.

## Stage 2 — Preamble & dependencies

Architectural decisions recorded for this stage:

- **Tag lookup:** static `const TAG_TABLE: &[(&str, Tag)]` with
  case-insensitive lookup via `eq_ignore_ascii_case`. Pre-pass for
  numbered tags strips the trailing digit run from `Source`/`Patch`/
  `NoSource`/`NoPatch` before lookup. Unknown names become
  `Tag::Other(<verbatim>)`.
- **`(qualifier)` vs `(lang)`:** decided by tag class.
  `Requires` / `Provides` / `Conflicts` / `Obsoletes` / `Recommends` /
  `Suggests` / `Supplements` / `Enhances` / `BuildConflicts` /
  `OrderWithRequires` accept comma-separated qualifiers; `Summary` /
  `Group` / `Description` accept a single locale token. For unknown
  tags we try qualifiers first, fall back to lang on parse failure.
- **Multi-dep splitter:** lives in `parser/preamble.rs`, not in
  `parser/deps.rs`. Splits on whitespace and commas at paren depth 0,
  so `Requires: (foo or bar) baz, quux` → 3 dep slices. One source
  line → N `PreambleItem`s sharing the same `Tag`.
- **EVR shape:** `[<digits>:]<version>[-<release>]`. The first `-`
  separates version and release (version is forbidden to contain `-`
  by rpm convention).
- **Arch qualifier heuristic:** the last trailing `(...)` of an atom
  name is treated as `arch` when its contents are `[A-Za-z0-9_-]+` and
  contain no `.` and no nested `(`. Otherwise it remains part of the
  name. So `kernel(x86-64)` → `name="kernel", arch="x86-64"`, while
  `pkgconfig(glib-2.0)` → `name="pkgconfig(glib-2.0)", arch=None`.
- **Rich/boolean deps:** strict recursive-descent. Every `(...)`
  introduces one frame. All operands on a level must share the *same*
  operator; mixed-precedence chains without explicit nesting raise an
  error diagnostic. Supported operators: `and`, `or`, `with`,
  `without`, `if … [else …]`, `unless … [else …]`.
- **`Tag` value dispatch:**
  - dep-bearing tags → split, then `DepExpr` each
  - `Epoch` → `TagValue::Number(u32)`
  - `AutoReq`/`AutoProv`/`AutoReqProv` → `Bool` (accept `0`/`1`/`yes`/
    `no`, case-insensitive)
  - `BuildArch`/`ExclusiveArch`/`ExcludeArch`/`ExclusiveOS`/`ExcludeOS`
    → `ArchList(Vec<Text>)`, whitespace-split, macros preserved
  - everything else → `Text` (parsed for macro segments)
- **Top-level loop additions:** insert *preamble line* before the
  unknown-line fallback. Distinguish from section header by leading
  character: `%` → section/cond/macro path; `[A-Za-z][A-Za-z0-9_]*:` →
  preamble path.
- **Section scope:** Stage 2 implements `%description` (body =
  `TextBody`) and `%package` (body = `Vec<PreambleContent<Span>>`).
  Other section headers remain deferred until Stage 3.

Modules to land:

- `parser/preamble.rs`:
  - `parse_preamble_line(state, input) -> IResult<Input, Vec<SpecItem<Span>>>`
    — returns N items for multi-dep lines, 1 item otherwise.
  - `parse_preamble_content(state, input) -> IResult<Input, PreambleContent<Span>>`
    — used inside `%package`; dispatches blank / comment / cond /
    preamble item.
  - `parse_package_body(state, input) -> IResult<Input, Vec<PreambleContent<Span>>>`
    — loops `parse_preamble_content` until next section header.
  - tag-lookup helpers (`lookup_tag`, `is_dep_tag`, `is_arch_list_tag`,
    `is_bool_tag`, `is_number_tag`, `is_lang_tag`).
  - qualifier vs lang resolver.
  - multi-dep splitter respecting paren depth.
- `parser/deps.rs`:
  - `parse_dep_expr(state, input: &str) -> Result<DepExpr, Diagnostic>`
    (string-based — no nom_locate needed for inside-token grammar).
  - sub-parsers: `parse_atom`, `parse_evr`, `parse_constraint`,
    `parse_rich` with mutually-recursive descent.
  - tokenizer for rich expressions (operators, idents, parens).
  - arch-heuristic helper.
- `parser/section.rs`:
  - `parse_section(state, input) -> IResult<Input, Section<Span>>`.
  - handlers for `%description` and `%package`; everything else
    delegates back to the deferred-placeholder path.
- `parser/entry.rs`:
  - extend `parse_top_level_item` / top-level loop to call
    `parse_preamble_line` and structural `parse_section` before the
    swallow-section fallback.
  - update `strip_section` to actually walk `Section<Span>` →
    `Section<()>` (it currently `unreachable!`s).

Stage 2 acceptance:

- A canonical Fedora preamble (Name, Version, Release, Summary,
  License, URL, BuildArch, Source0, Patch0, BuildRequires, Requires)
  parses without warnings; tag-by-tag assertion in an integration
  test.
- `%package -n foo` with its own preamble and `%description -n foo`
  parses; subpackage content is captured under `Section::Package`.
- Rich deps round-trip through debug-format equality:
  `Requires: (foo and bar)`, `Requires: (a if b else c)`,
  `Requires: (a without b)`.
- File deps as atoms: `Requires: /usr/bin/awk` →
  `DepAtom { name: Text("/usr/bin/awk"), arch: None, … }`.
- Multi-dep splitter: `Requires: foo bar, baz` → 3 items.
- Multi-line preamble values via `\`: `Description: line1 \⏎line2`
  collapses correctly (kept as one `Text`).
- Arch heuristic exercised on both `kernel(x86-64)` and
  `pkgconfig(glib-2.0)`.

Out of scope for Stage 2 (still deferred):

- `%files` directives (Stage 3).
- `%changelog` entries (Stage 3).
- `%pre`/`%post`/`%trigger*` scriptlets (Stage 3).
- `%bcond` semantic evaluation (Stage 4 / validator).

## Stage 3 — Section bodies

Architectural decisions recorded for this stage:

- **Section header forms:** every section accepts `-n NAME`
  (`SubpkgRef::Absolute`) and bare `NAME` (`SubpkgRef::Relative`),
  identical to `%description`/`%package` from Stage 2.
- **`%if`/`%endif` inside `%files`:** structural —
  `FilesContent::Conditional(Conditional<Span, FilesContent<Span>>)`,
  reusing the generic `parse_conditional`.
- **`%if`/`%endif` inside shell bodies (`%prep`/`%build`/scriptlets/
  triggers/`%verify`/`%sepolicy`):** *not* structural — they remain
  text lines in `ShellBody::lines`. AST contract from Stage 1.
- **Shell-body helper:** one `collect_shell_body_until_section_header`
  in `parser/section.rs`, shared by `%prep`/`%conf`/`%build`/`%install`/
  `%check`/`%clean`/`%generate_buildrequires`, all scriptlets,
  triggers, `%verify`, `%sepolicy`.
- **File directive grammar:** whitelisted names (`%attr`/`%defattr`/
  `%dir`/`%doc`/`%license`/`%config`/`%ghost`/`%verify`/`%lang`/`%caps`/
  `%artifact`/`%missingok`). Anything starting with `%xyz` where `xyz`
  is *not* in this whitelist is treated as the start of the file path
  (a macro reference inside the path).
- **Path vs directive split:** after consuming any leading directives
  with their `(...)` arguments and whitespace, the rest of the line is
  the path (whitespace allowed inside paths — rpm does not require
  escaping). One physical line = one `FileEntry`.
- **Scriptlet header:** options in any order — `-n NAME`, `-p INTERP`,
  `-e`, `-q`, `-f FILE`. A bare first token before any flag is taken as
  a relative subpkg name. Triggers add `--` then comma-separated
  conditions (parsed by `parse_dep_expr`). File-triggers add
  `-P PRIORITY` and `--` then path prefixes.
- **`-p <lua>` interpreter:** literal `<lua>` → `Interpreter::Lua`.
  Anything else after `-p` → `Interpreter::Path(Text)`.
- **Changelog entry header:**
  `* Weekday Month Day Year Author [<email>] [- VERSION[-RELEASE]]`.
  Email is taken from the first `<…>` token in the author segment.
  The trailing `- …` (preceded by whitespace + `-` + whitespace) is
  optional and stored as `version: Option<Text>` (parsed for macros).
  Body lines are `Vec<Text>` (macros parsed) up to the next `*`-headed
  entry or the next section header.
- **`%sourcelist` / `%patchlist`:** one `Text` per non-blank,
  non-comment body line (each is a Source/Patch entry without a
  numeric index).

Modules to land:

- `parser/section.rs` — new public helpers:
  - `pub(crate) fn collect_shell_body_until_section_header(state,
    input) -> (Input, ShellBody)` — used by every shell-body section.
  - extended `parse_section` dispatch (all section header names).
  - small handlers inline for trivial cases: `%verify`/`%sepolicy`
    (just header args + shell body), `%sourcelist`/`%patchlist` (one
    `Text` per body line).
  - build-script handler: maps section header to `BuildScriptKind`,
    body via the shell helper.
- `parser/files.rs` — *new*:
  - `parse_files_section(state, input) -> IResult<Input, Section<Span>>`.
  - `parse_files_content(state, input) -> IResult<Input, Vec<FilesContent<Span>>>`
    (the body-item parser passed to `parse_conditional`).
  - `parse_file_entry(state, input) -> IResult<Input, FileEntry<Span>>`.
  - `parse_directive` for each of the whitelisted names; each returns
    `FileDirective`.
  - `parse_attr_field` for `%attr(...)`/`%defattr(...)` fields
    (numeric octal / `-` / Text name).
- `parser/changelog.rs` — *new*:
  - `parse_changelog_section(state, input) -> IResult<Input, Section<Span>>`.
  - `parse_changelog_entry(state, input) -> IResult<Input, ChangelogEntry<Span>>`.
  - `parse_date_header` (weekday/month/day/year + author/email/version
    extraction).
  - `parse_weekday`/`parse_month` lookup against fixed 3-letter table.
- `parser/scriptlet.rs` — *new*:
  - `parse_scriptlet_section(state, input, kind) -> IResult<Input, Section<Span>>`.
  - `parse_trigger_section(state, input, kind) -> IResult<Input, Section<Span>>`.
  - `parse_file_trigger_section(state, input, kind) -> IResult<Input, Section<Span>>`.
  - `parse_scriptlet_header(state, input) -> ScriptletHeader` —
    consumes options in any order, returns a structured opts bundle.
  - `parse_interpreter` (handles literal `<lua>`).
- `parser/entry.rs`:
  - `strip_section` extended to handle every `Section<Span>` variant
    (currently only `Description` and `Package`).

Stage 3 acceptance:

- A canonical Fedora-like spec end-to-end parses to a fully structural
  AST: preamble + `%description` + `%package` + `%prep` + `%build` +
  `%install` + `%check` + `%files` + `%post` (with `-p` and bare
  subpkg) + `%triggerin` + `%changelog`. Integration test
  `tests/parser_stage3.rs` asserts node counts per section type and
  inspects a representative directive/entry/scriptlet.
- File directives covered by unit tests: `%defattr(-,root,root,-)`,
  `%attr(0755, root, root)`, `%config(noreplace) /etc/foo`,
  `%verify(not md5 size mtime)`, `%lang(ru_RU)`, `%caps(cap_…)`,
  `%dir`, `%doc`, `%license`, `%ghost`, `%artifact`, `%missingok`.
- Multiple directives on one line: `%attr(0755,root,root) %config /etc/foo`.
- `%if` inside `%files` parses as `FilesContent::Conditional`.
- Changelog: two entries with multi-line bodies; email extraction;
  trailing version.
- Scriptlet `%post -p /sbin/ldconfig` with empty body; `%post libfoo`
  with `Relative` subpkg; `%post -p <lua>` with `Lua` interpreter;
  trigger `%triggerin -- foo, bar` with two dep conditions.
- No more "deferred" diagnostics emitted on a complete canonical spec.

## Stage 4 — Parser-side stabilization

Architectural decisions recorded for this stage:

- **Scope:** parser-side stabilization *only*. Printer + roundtrip are
  Stage 5.
- **Diagnostic code namespace:** `rpmspec/E####` for errors,
  `rpmspec/W####` for warnings. Constants live in
  `parse_result::codes`; each is `pub const … : &'static str`. Every
  existing `push_warning`/`push_error` site is rewritten to pass a code.
- **Tracing:** optional feature `tracing` (workspace dep
  `tracing = { version = "0.1", default-features = false, optional }`).
  Instrumentation only on public entry points `parse_str` and
  `parse_str_with_spans` via
  `#[cfg_attr(feature = "tracing", tracing::instrument(...))]`. No
  per-sub-parser noise.
- **Send + Sync proof:** new `tests/auto_traits.rs` asserts at compile
  time that `SpecFile<()>`, `SpecFile<Span>`, `ParseResult<()>`,
  `ParseResult<Span>`, `ParseError`, `PrintError`, `Diagnostic` are
  `Send + Sync`.
- **`ParseError` finalization:** existing 4 variants
  (`Io`/`Syntax`/`UnterminatedConditional`/`InvalidSection`) reviewed;
  `#[non_exhaustive]` already set; no new variants added unless a
  concrete parser site needs one.
- **Doc lint strategy:** start with `#![warn(missing_docs)]` to surface
  the gaps, write the missing docs, finish by flipping to
  `#![deny(missing_docs)]` only after the warnings are clean.
- **API helpers on `ParserState`:** add `push_warning_code` /
  `push_error_code` that take `code: &'static str` plus message + span,
  so every diagnostic site is concise: `state.push_warning_code(
  codes::W_STRAY_PERCENT, "...", Some(span));`.

Files to land:

- `crates/rpm-spec/src/parse_result.rs` — `pub mod codes` with the
  full enum of stable diagnostic codes; doc-comments on each.
- `crates/rpm-spec/src/parser/state.rs` — `push_warning_code` /
  `push_error_code` helpers.
- `Cargo.toml` (workspace + member) — `tracing` optional dep + feature.
- `crates/rpm-spec/src/parser/entry.rs` —
  `#[cfg_attr(feature = "tracing", tracing::instrument(...))]` on
  `parse_str` and `parse_str_with_spans`. `tracing::Level::DEBUG`,
  fields `input_len`, skip `input`.
- All parser modules — replace bare `push_warning`/`push_error` with
  `push_warning_code(codes::W_…, …)` / `push_error_code(codes::E_…, …)`.
- `crates/rpm-spec/src/lib.rs` — first `#![warn(missing_docs)]`, then
  flip to `#![deny(missing_docs)]` once the pass is complete.
- Doc pass over: `ast::*` (every pub type + field), `parser::*`
  (every pub fn), `parse_result::{ParseResult, Diagnostic, Severity,
  codes}`, `error::{ParseError, PrintError}`.
- `crates/rpm-spec/tests/auto_traits.rs` — compile-time Send/Sync.

Stage 4 acceptance:

- `cargo test --all-features` green (all 187 tests + new auto_traits).
- `cargo clippy --all-features --all-targets -- -D warnings` clean.
- `cargo doc --no-deps --all-features` clean, with
  `#![deny(missing_docs)]` enabled in `lib.rs`.
- Every `Diagnostic` produced on the canonical Stage 3 spec carries a
  non-`None` `code`.
- `cargo check --all-features --no-default-features --features tracing`
  compiles.

## Stage 5 — Pretty-printer & roundtrip

Architectural decisions recorded for this stage:

- **Engine:** simple recursive writer over `&mut String`, no
  `pretty::DocBuilder`. Spec values are single-line or use explicit
  `\`-continuation; column-aware wrapping is unnecessary.
- **API:**
  - `pub fn print<T>(spec: &SpecFile<T>) -> String`
  - `pub fn print_with<T>(spec: &SpecFile<T>, cfg: &PrinterConfig) -> String`
  - `String` (not `Result`): the AST is already valid, printer cannot
    fail fatally.
- **`PrinterConfig` fields (with builder methods):**
  - `indent: usize` — spaces per nesting level inside `Conditional`
    blocks. Default `0` (flush-left).
  - `preamble_value_column: Option<usize>` — column to align preamble
    values at. Default `Some(16)` (Fedora convention). If the
    `Tag(qual):` prefix is already wider, fall back to single space.
- **Indent scope:** every nested `Conditional` (top-level, inside
  `%package`, inside `%files`) gets `+config.indent` spaces per
  level. Section bodies and shell bodies themselves stay flush-left.
- **`%if` inside shell bodies:** untouched — they live as plain
  `ShellBody::lines` text and the printer emits them verbatim
  (printer has no structural information there).
- **Multi-dep collapse:** *not* performed. Each `PreambleItem`
  renders to its own line. Lossy with respect to source (one input
  line may become N output lines), gains pretty-print readability;
  follows Fedora style guidance.
- **`%%` escaping:** every literal `%` inside `TextSegment::Literal`
  is emitted as `%%`. This keeps `Text::from("50%")` round-trippable
  through `print → parse`.
- **Blank-line policy:** insert one blank line before every
  `SpecItem::Section` except the first; preserve any
  `SpecItem::Blank` items from source. No automatic blanks between
  consecutive `Preamble` items.

Note on round-trip with `indent > 0`: rpm itself does not parse
indented `%if` blocks, but **this crate's parser** does (every
section header / cond keyword / macro statement is preceded by
`space0`). So `parse → print(indent=N) → parse` is consistent for
in-tool round-trip; the output is not meant to be fed back to rpm.

Modules to land:

- `printer/mod.rs` — entry points + `PrinterConfig` + builder methods
  + `Printer` context (`&mut String` + `indent_level: usize` +
  `nested(...)` helper).
- `printer/text.rs` — `Text` / `TextSegment` / `MacroRef` rendering;
  `%%`-escape literal `%`; every macro kind (Plain, Braced,
  Parametric, Shell, Expr, Lua, Builtin, conditional prefix,
  `with_value`).
- `printer/deps.rs` — `DepExpr`/`DepAtom`/`BoolDep` rendering;
  recursive rich-dep emission with explicit parens at every level.
- `printer/preamble.rs` — `Tag: value` lines with
  `preamble_value_column` alignment; qualifier / lang formatting.
- `printer/section.rs` — `Section` dispatch + section headers +
  `ShellBody` line emission.
- `printer/files.rs` — `FilesContent` body + directives.
- `printer/changelog.rs` — entry header + body lines.
- `printer/scriptlet.rs` — scriptlet/trigger/file-trigger headers
  with option ordering matching parser's accepted forms.
- `printer/cond.rs` — `Conditional` with indent application.
- `printer/macros.rs` — `MacroDef`/`BuildCondition`/`IncludeDirective`/
  `Comment` rendering; multi-line `%define` body re-emits trailing
  `\` for source-line breaks.

Stage 5 acceptance:

- `cargo test --all-features` green; new printer tests per module +
  integration `tests/roundtrip.rs`.
- Round-trip on the Stage 3 canonical spec:
  `parse_str → print → parse_str → assert structural equality`.
  Multi-dep collapse is the only source of *intentional* divergence.
- Indent-`N` round-trip:
  `parse_str → print_with(indent=2) → parse_str → same items count
  and structure`.
- Manual eyeball: `print` output of the Stage 3 canonical spec looks
  clean (proper alignment, blank lines, indentation).

## Backlog (anyone of these can promote out of backlog as needed)

- Compact strings (`compact_str` or `Box<str>`) on hot fields once the
  parser is profiled.
- `%macro_setup`/`%autosetup`/`%patch -P N` shape-aware sub-parsing
  inside `%prep` body (still kept as `Text`, but with `MacroRef` args
  parsed).
- `%bcond` boolean evaluation pass (separate skill — belongs in
  `rpm-spec-validator`).
- Macro expansion pass (separate skill).
- Streaming/Incremental parse hooks for LSP.
