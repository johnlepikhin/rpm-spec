//! `%changelog` entry rendering.

use crate::ast::{ChangelogEntry, Month, Section, Span, Weekday};

use super::Printer;
use super::text::print_text;

/// Render a `Section::Changelog` body (header line + entries).
pub(crate) fn print_changelog<T>(p: &mut Printer<'_>, entries: &[ChangelogEntry<T>]) {
    p.line("%changelog");
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            p.newline();
        }
        print_entry(p, entry);
    }
}

fn print_entry<T>(p: &mut Printer<'_>, e: &ChangelogEntry<T>) {
    p.write_indent();
    p.raw_char('*');
    p.raw_char(' ');
    p.raw(weekday_str(e.date.weekday));
    p.raw_char(' ');
    p.raw(month_str(e.date.month));
    p.raw_char(' ');
    p.raw(&format!("{:02}", e.date.day));
    p.raw_char(' ');
    p.raw(&e.date.year.to_string());
    p.raw_char(' ');
    print_text(p, &e.author);
    if let Some(email) = &e.email {
        p.raw_char(' ');
        p.raw_char('<');
        print_text(p, email);
        p.raw_char('>');
    }
    if let Some(version) = &e.version {
        p.raw(" - ");
        print_text(p, version);
    }
    p.newline();
    for line in &e.body {
        p.write_indent();
        print_text(p, line);
        p.newline();
    }
}

fn weekday_str(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn month_str(m: Month) -> &'static str {
    match m {
        Month::Jan => "Jan",
        Month::Feb => "Feb",
        Month::Mar => "Mar",
        Month::Apr => "Apr",
        Month::May => "May",
        Month::Jun => "Jun",
        Month::Jul => "Jul",
        Month::Aug => "Aug",
        Month::Sep => "Sep",
        Month::Oct => "Oct",
        Month::Nov => "Nov",
        Month::Dec => "Dec",
    }
}

/// Helper so `section.rs` can route a `Section::Changelog` here.
pub(crate) fn print_section_changelog<T>(p: &mut Printer<'_>, entries: &[ChangelogEntry<T>]) {
    print_changelog(p, entries);
}

// Suppress "unused imports" if Section/Span happen to be unreferenced
// in this small module.
#[allow(dead_code)]
fn _suppress_unused(_: Option<&Section<Span>>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ChangelogDate, Text};
    use crate::printer::PrinterConfig;

    fn entry() -> ChangelogEntry<()> {
        ChangelogEntry {
            date: ChangelogDate {
                weekday: Weekday::Wed,
                month: Month::May,
                day: 14,
                year: 2025,
            },
            author: Text::from("Maintainer"),
            email: Some(Text::from("m@example.org")),
            version: Some(Text::from("1.0-1")),
            body: vec![Text::from("- initial packaging")],
            data: (),
        }
    }

    fn render(entries: &[ChangelogEntry<()>]) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_changelog(&mut p, entries);
        buf
    }

    #[test]
    fn single_entry() {
        let out = render(&[entry()]);
        assert_eq!(
            out,
            "%changelog\n* Wed May 14 2025 Maintainer <m@example.org> - 1.0-1\n- initial packaging\n"
        );
    }

    #[test]
    fn two_entries_separated_by_blank() {
        let mut e2 = entry();
        e2.date.year = 2024;
        e2.body = vec![Text::from("- older")];
        let out = render(&[entry(), e2]);
        assert!(out.contains("\n\n* Wed May 14 2024"));
    }

    #[test]
    fn no_email_no_version() {
        let mut e = entry();
        e.email = None;
        e.version = None;
        let out = render(&[e]);
        assert!(out.contains("Maintainer\n"));
    }
}
