//! Pretty-printer that renders a [`crate::ast::SpecFile`] back into spec
//! source text.
//!
//! # Output shape
//!
//! - Preamble values are aligned at column
//!   [`PrinterConfig::preamble_value_column`] (default 16).
//! - Top-level sections are separated by a single blank line.
//! - Inside `Conditional` blocks every nested level is indented by
//!   `[PrinterConfig::indent]` spaces. Default `0` keeps `%if`/`%endif`
//!   flush-left, mimicking idiomatic spec source. Non-zero values
//!   improve readability of deeply nested conditionals.
//!
//! # Round-trip
//!
//! This crate's parser tolerates leading whitespace before every
//! section header, macro statement, and `%if`/`%endif` keyword, so
//! `parse → print(indent=N) → parse` is internally consistent. The
//! output is **not** guaranteed to be accepted by `rpmbuild` itself
//! when `indent > 0`: rpm's own parser rejects indented conditionals.

mod changelog;
mod cond;
mod deps;
mod expr;
mod files;
mod macros;
mod preamble;
mod scriptlet;
mod section;
mod text;
mod util;

use crate::ast::{Section, SpecFile, SpecItem};

// ---------------------------------------------------------------------
// Output abstraction
// ---------------------------------------------------------------------

/// Source-level token category used by the pretty-printer to classify
/// each emitted chunk. Consumers that care about syntax highlighting
/// (e.g. the `pretty` CLI subcommand) implement [`PrintWriter`] and
/// dispatch on this kind; the default [`String`] implementation
/// ignores the category and concatenates verbatim, preserving the
/// round-trip invariant.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Whitespace, punctuation, indentation — everything that has no
    /// independent semantic value.
    Plain,
    /// `Name`, `Version`, `Requires`, … — left-hand side of a preamble item.
    TagName,
    /// `(post,postun)` qualifier tail on a tag.
    TagQualifier,
    /// `%description`, `%files`, `%prep`, `%build`, `%install`, `%changelog`, etc.
    SectionKeyword,
    /// `%if`, `%elif`, `%else`, `%endif`, `%ifarch`, `%ifos`, `%ifnarch`, `%ifnos`.
    ConditionalKeyword,
    /// `%define`, `%global`, `%undefine`, `%include`, `%bcond*`.
    MacroDefKeyword,
    /// `%{name}`, `%name`, `%{?name}`, `%{!?name}` — macro references in any position.
    MacroRef,
    /// `%(...)` shell substitution.
    ShellMacro,
    /// `%[...]` and `%{lua:...}` expression macros.
    ExprMacro,
    /// Quoted string literal inside `%if` expressions.
    String,
    /// Integer literal inside `%if` expressions.
    Number,
    /// `&&`, `||`, `==`, `!=`, `>=`, `>`, `<=`, `<`, `!`.
    Operator,
    /// `#` comments and `%dnl` directives.
    Comment,
    /// `* Mon Jan 01 2024 …` header line of a changelog entry.
    ChangelogHeader,
    /// Free-form shell body inside `%prep`/`%build`/`%post`/etc.
    ShellBody,
    /// Free-form text body inside `%description` and changelog entries.
    TextBody,
    /// Modifier flags on macro definitions and tags (e.g. `-e`, `-g`).
    Flag,
}

/// Sink for printer output. Consumers decide how to render each
/// [`TokenKind`]; the default [`String`] implementation ignores the
/// category and concatenates verbatim, so existing
/// [`print()`] / [`print_with`] callers see byte-identical output.
///
/// # Design: infallibility
///
/// [`PrintWriter::emit`] returns `()`. The trait targets in-memory
/// sinks (String/Vec buffers, ANSI highlighters that buffer
/// internally) and is **not** suitable for fallible writers such as
/// `std::io::Write` over a network socket or a closed pipe.
/// Adapters over fallible writers should buffer errors internally
/// (e.g. store the first `io::Error` in `&mut self`) and surface
/// them after [`print_to`] returns. A future major release may
/// introduce a fallible variant.
///
/// # Examples
///
/// ANSI-style classifier that wraps each chunk in its category name —
/// useful when piping into a downstream colorizer:
///
/// ```
/// use rpm_spec::printer::{PrintWriter, TokenKind};
///
/// #[derive(Default)]
/// struct Tagged(String);
///
/// impl PrintWriter for Tagged {
///     fn emit(&mut self, kind: TokenKind, text: &str) {
///         self.0.push_str(&format!("[{:?}:{}]", kind, text));
///     }
/// }
/// ```
pub trait PrintWriter {
    /// Emit a chunk of source text classified as `kind`.
    fn emit(&mut self, kind: TokenKind, text: &str);
}

impl PrintWriter for String {
    fn emit(&mut self, _kind: TokenKind, text: &str) {
        self.push_str(text);
    }
}

/// Default preamble-value alignment column matching Fedora packaging
/// style ("Name:           value", value at column 16).
pub const FEDORA_PREAMBLE_VALUE_COLUMN: usize = 16;

/// Configuration knobs for the pretty-printer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterConfig {
    /// Spaces added per nesting level inside `Conditional` blocks. `0`
    /// (the default) keeps `%if` keyword flush-left, matching rpm
    /// conventions.
    pub indent: usize,
    /// Column at which preamble values are aligned. Default
    /// [`Some(FEDORA_PREAMBLE_VALUE_COLUMN)`]; if a tag's
    /// `Tag(qualifier):` prefix is already wider, a single space is
    /// used instead. `None` always uses a single space.
    pub preamble_value_column: Option<usize>,
}

impl Default for PrinterConfig {
    fn default() -> Self {
        Self {
            indent: 0,
            preamble_value_column: Some(FEDORA_PREAMBLE_VALUE_COLUMN),
        }
    }
}

impl PrinterConfig {
    /// Build a default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the nested-indent width (spaces per level inside
    /// `Conditional` blocks).
    #[must_use]
    pub fn with_indent(mut self, spaces: usize) -> Self {
        self.indent = spaces;
        self
    }

    /// Set the preamble value alignment column.
    #[must_use]
    pub fn with_preamble_value_column(mut self, col: Option<usize>) -> Self {
        self.preamble_value_column = col;
        self
    }
}

/// Render a [`SpecFile`] with default configuration into a [`String`].
pub fn print<T>(spec: &SpecFile<T>) -> String {
    print_with(spec, &PrinterConfig::default())
}

/// Render a [`SpecFile`] with explicit configuration into a [`String`].
pub fn print_with<T>(spec: &SpecFile<T>, cfg: &PrinterConfig) -> String {
    let mut out = String::new();
    print_to(spec, cfg, &mut out);
    out
}

/// Render a [`SpecFile`] into any [`PrintWriter`]. This is the
/// extension point for consumers (e.g. ANSI highlighters) that want
/// to route each chunk through a category-aware sink. The default
/// [`String`] implementation gives byte-identical output to
/// [`print_with`].
///
/// # Examples
///
/// ```
/// use rpm_spec::ast::SpecFile;
/// use rpm_spec::printer::{print_to, PrinterConfig};
///
/// let spec: SpecFile<()> = SpecFile::default();
/// let mut out = String::new();
/// print_to(&spec, &PrinterConfig::default(), &mut out);
/// assert_eq!(out, "");
/// ```
pub fn print_to<T>(
    spec: &SpecFile<T>,
    cfg: &PrinterConfig,
    w: &mut dyn PrintWriter,
) {
    let mut p = Printer::new(w, cfg);
    print_spec(&mut p, spec);
}

/// Two consecutive `\n` bytes mean the next emit would start on a
/// blank line — used by [`Printer::ends_with_blank_line`] to decide
/// when section headers need an extra newline before them.
const BLANK_LINE_NEWLINES: u8 = 2;

// ---------------------------------------------------------------------
// Printer context
// ---------------------------------------------------------------------

/// Internal writer state. Carries the output sink, the active config,
/// the current indent level, and bookkeeping for detecting
/// "have we just emitted a blank line" — required for section spacing
/// without needing to peek at the buffer (the writer is opaque).
pub(crate) struct Printer<'a> {
    out: &'a mut dyn PrintWriter,
    cfg: &'a PrinterConfig,
    indent_level: usize,
    /// Number of `\n` bytes at the very end of the output stream
    /// (saturating at 2 — that's all we need to detect blank lines).
    trailing_newlines: u8,
    /// `true` once any byte has been emitted. An untouched output is
    /// treated as ending with a blank line — so the first section
    /// header doesn't get a leading newline.
    emitted: bool,
}

impl<'a> Printer<'a> {
    pub(crate) fn new(out: &'a mut dyn PrintWriter, cfg: &'a PrinterConfig) -> Self {
        Self {
            out,
            cfg,
            indent_level: 0,
            trailing_newlines: 0,
            emitted: false,
        }
    }

    /// Reference the active config.
    pub(crate) fn cfg(&self) -> &PrinterConfig {
        self.cfg
    }

    /// Emit `text` classified as `kind` and update trailing-newline
    /// bookkeeping. All other emit helpers funnel through here.
    pub(crate) fn emit(&mut self, kind: TokenKind, text: &str) {
        if text.is_empty() {
            return;
        }
        self.emitted = true;
        let new_trailing = text.bytes().rev().take_while(|&b| b == b'\n').count();
        // `new_trailing` is bounded by `text.len()` but we only care up to 2.
        let added = new_trailing.min(BLANK_LINE_NEWLINES as usize) as u8;
        self.trailing_newlines = if new_trailing == text.len() {
            self.trailing_newlines.saturating_add(added).min(BLANK_LINE_NEWLINES)
        } else {
            added
        };
        self.out.emit(kind, text);
    }

    /// Append a chunk classified as [`TokenKind::Plain`] — the default
    /// for whitespace and other neutral punctuation.
    pub(crate) fn raw(&mut self, s: &str) {
        self.emit(TokenKind::Plain, s);
    }

    /// Append a single plain character.
    pub(crate) fn raw_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.emit(TokenKind::Plain, c.encode_utf8(&mut buf));
    }

    /// Emit the current line's indentation prefix
    /// (`cfg.indent * indent_level` spaces).
    pub(crate) fn write_indent(&mut self) {
        let n = self.cfg.indent.saturating_mul(self.indent_level);
        if n == 0 {
            return;
        }
        // Reuse a small stack buffer for the common case; fall back to
        // heap for absurdly large indents.
        const STACK: &str =
            "                                                                ";
        if n <= STACK.len() {
            self.raw(&STACK[..n]);
        } else {
            self.raw(&" ".repeat(n));
        }
    }

    /// Append `\n`.
    pub(crate) fn newline(&mut self) {
        self.raw("\n");
    }

    /// Run `body` with the indent level temporarily increased by one.
    pub(crate) fn nested<F: FnOnce(&mut Self)>(&mut self, body: F) {
        self.indent_level += 1;
        body(self);
        self.indent_level -= 1;
    }

    /// `true` when nothing has been emitted yet, or the last two
    /// bytes were `\n` — i.e. the next chunk would start on a blank
    /// line.
    pub(crate) fn ends_with_blank_line(&self) -> bool {
        !self.emitted || self.trailing_newlines >= BLANK_LINE_NEWLINES
    }
}

// ---------------------------------------------------------------------
// Top-level driver
// ---------------------------------------------------------------------

fn print_spec<T>(p: &mut Printer<'_>, spec: &SpecFile<T>) {
    for (idx, item) in spec.items.iter().enumerate() {
        let needs_blank_before = matches!(item, SpecItem::Section(_)) && idx > 0;
        if needs_blank_before && !p.ends_with_blank_line() {
            p.newline();
        }
        print_spec_item(p, item);
    }
}

pub(crate) fn print_spec_item<T>(p: &mut Printer<'_>, item: &SpecItem<T>) {
    match item {
        SpecItem::Preamble(pi) => preamble::print_preamble_item(p, pi),
        SpecItem::Section(sec) => print_section(p, sec.as_ref()),
        SpecItem::Conditional(c) => {
            cond::print_conditional(p, c, |p, it| print_spec_item(p, it));
        }
        SpecItem::MacroDef(m) => macros::print_macro_def(p, m),
        SpecItem::BuildCondition(b) => macros::print_build_condition(p, b),
        SpecItem::Include(i) => macros::print_include(p, i),
        SpecItem::Statement(m) => {
            p.write_indent();
            // Render the whole `%macro …` statement to a side buffer
            // and emit it as a single classified chunk. Categorize as
            // `MacroRef` — these are bare macro invocations like
            // `%setup`, `%patch0`, `%autosetup`, etc.
            let mut buf = String::new();
            {
                let mut tmp = Printer::new(&mut buf, p.cfg());
                tmp.raw_char('%');
                text::print_macro_ref_no_percent(&mut tmp, m);
            }
            p.emit(TokenKind::MacroRef, &buf);
            p.newline();
        }
        SpecItem::Comment(c) => macros::print_comment(p, c),
        SpecItem::Blank => {
            p.newline();
        }
    }
}

fn print_section<T>(p: &mut Printer<'_>, section: &Section<T>) {
    section::print_section(p, section);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{PreambleItem, Tag, TagValue, Text};

    #[test]
    fn default_config_is_no_indent_col16() {
        let cfg = PrinterConfig::default();
        assert_eq!(cfg.indent, 0);
        assert_eq!(cfg.preamble_value_column, Some(16));
    }

    #[test]
    fn builders_compose() {
        let cfg = PrinterConfig::new()
            .with_indent(4)
            .with_preamble_value_column(None);
        assert_eq!(cfg.indent, 4);
        assert!(cfg.preamble_value_column.is_none());
    }

    #[test]
    fn empty_spec_yields_empty_string() {
        let spec: SpecFile<()> = SpecFile::default();
        assert_eq!(print(&spec), "");
    }

    #[test]
    fn single_preamble_item() {
        let mut spec: SpecFile<()> = SpecFile::default();
        spec.items.push(SpecItem::Preamble(PreambleItem {
            tag: Tag::Name,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Text(Text::from("hello")),
            data: (),
        }));
        let out = print(&spec);
        assert!(out.starts_with("Name:"));
        assert!(out.ends_with("hello\n"));
    }
}
