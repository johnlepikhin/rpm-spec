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

mod cond;
mod deps;
mod files;
mod macros;
mod preamble;
mod scriptlet;
mod section;
mod text;
mod changelog;

use crate::ast::{Section, SpecFile, SpecItem};

/// Configuration knobs for the pretty-printer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterConfig {
    /// Spaces added per nesting level inside `Conditional` blocks. `0`
    /// (the default) keeps `%if` keyword flush-left, matching rpm
    /// conventions.
    pub indent: usize,
    /// Column at which preamble values are aligned. `Some(16)` matches
    /// Fedora packaging style; if a tag's `Tag(qualifier):` prefix is
    /// already wider, a single space is used instead. `None` always
    /// uses a single space.
    pub preamble_value_column: Option<usize>,
}

impl Default for PrinterConfig {
    fn default() -> Self {
        Self {
            indent: 0,
            preamble_value_column: Some(16),
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

/// Render a [`SpecFile`] with default configuration.
pub fn print<T>(spec: &SpecFile<T>) -> String {
    print_with(spec, &PrinterConfig::default())
}

/// Render a [`SpecFile`] with explicit configuration.
pub fn print_with<T>(spec: &SpecFile<T>, cfg: &PrinterConfig) -> String {
    let mut out = String::new();
    let mut p = Printer::new(&mut out, cfg);
    print_spec(&mut p, spec);
    out
}

// ---------------------------------------------------------------------
// Printer context
// ---------------------------------------------------------------------

/// Internal writer state. Carries the output buffer, the active config,
/// and the current indent level.
pub(crate) struct Printer<'a> {
    out: &'a mut String,
    cfg: &'a PrinterConfig,
    indent_level: usize,
}

impl<'a> Printer<'a> {
    pub(crate) fn new(out: &'a mut String, cfg: &'a PrinterConfig) -> Self {
        Self {
            out,
            cfg,
            indent_level: 0,
        }
    }

    /// Reference the active config.
    pub(crate) fn cfg(&self) -> &PrinterConfig {
        self.cfg
    }

    /// Append a raw string with no indentation prefix.
    pub(crate) fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// Append a single character.
    pub(crate) fn raw_char(&mut self, c: char) {
        self.out.push(c);
    }

    /// Emit the current line's indentation prefix
    /// (`cfg.indent * indent_level` spaces).
    pub(crate) fn write_indent(&mut self) {
        let n = self.cfg.indent.saturating_mul(self.indent_level);
        for _ in 0..n {
            self.out.push(' ');
        }
    }

    /// Append `\n`.
    pub(crate) fn newline(&mut self) {
        self.out.push('\n');
    }

    /// Append indent + content + `\n`.
    pub(crate) fn line(&mut self, content: &str) {
        self.write_indent();
        self.raw(content);
        self.newline();
    }

    /// Run `body` with the indent level temporarily increased by one.
    pub(crate) fn nested<F: FnOnce(&mut Self)>(&mut self, body: F) {
        self.indent_level += 1;
        body(self);
        self.indent_level -= 1;
    }

}

// ---------------------------------------------------------------------
// Top-level driver
// ---------------------------------------------------------------------

fn print_spec<T>(p: &mut Printer<'_>, spec: &SpecFile<T>) {
    for (idx, item) in spec.items.iter().enumerate() {
        let needs_blank_before = matches!(item, SpecItem::Section(_)) && idx > 0;
        if needs_blank_before && !ends_with_blank_line(p.out) {
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
            p.raw_char('%');
            text::print_macro_ref_no_percent(p, m);
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

fn ends_with_blank_line(s: &str) -> bool {
    s.is_empty() || s.ends_with("\n\n")
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
