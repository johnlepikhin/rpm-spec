//! Preamble item rendering.

use crate::ast::{
    PreambleContent, PreambleItem, Tag, TagQualifier, TagValue, Text, TextSegment,
};

use super::Printer;
use super::cond::print_conditional;
use super::deps::print_dep_expr;
use super::macros::print_comment;
use super::text::print_text;

pub(crate) fn print_preamble_item<T>(p: &mut Printer<'_>, item: &PreambleItem<T>) {
    p.write_indent();

    // Build the `Tag(qualifier):` (or `Tag(lang):` / `Tag:`) prefix.
    let prefix = tag_prefix(&item.tag, &item.qualifiers, item.lang.as_deref());
    p.raw(&prefix);

    let column_after_prefix = p.cfg().preamble_value_column.map(|target| {
        if prefix.len() < target {
            target - prefix.len()
        } else {
            1
        }
    });
    let pad = column_after_prefix.unwrap_or(1);
    for _ in 0..pad {
        p.raw_char(' ');
    }

    print_tag_value(p, &item.value);
    p.newline();
}

pub(crate) fn print_preamble_content<T>(p: &mut Printer<'_>, c: &PreambleContent<T>) {
    match c {
        PreambleContent::Item(it) => print_preamble_item(p, it),
        PreambleContent::Conditional(cond) => {
            print_conditional(p, cond, |p, body| print_preamble_content(p, body))
        }
        PreambleContent::Comment(cm) => print_comment(p, cm),
        PreambleContent::Blank => p.newline(),
    }
}

fn tag_prefix(tag: &Tag, qualifiers: &[TagQualifier], lang: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(&tag_name(tag));
    if !qualifiers.is_empty() {
        out.push('(');
        for (i, q) in qualifiers.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(qualifier_name(q));
        }
        out.push(')');
    } else if let Some(l) = lang {
        out.push('(');
        out.push_str(l);
        out.push(')');
    }
    out.push(':');
    out
}

fn tag_name(tag: &Tag) -> String {
    match tag {
        Tag::Name => "Name".into(),
        Tag::Version => "Version".into(),
        Tag::Release => "Release".into(),
        Tag::Summary => "Summary".into(),
        Tag::License => "License".into(),
        Tag::URL => "URL".into(),
        Tag::Group => "Group".into(),
        Tag::Epoch => "Epoch".into(),
        Tag::Icon => "Icon".into(),
        Tag::Source(None) => "Source".into(),
        Tag::Source(Some(n)) => format!("Source{n}"),
        Tag::Patch(None) => "Patch".into(),
        Tag::Patch(Some(n)) => format!("Patch{n}"),
        Tag::NoSource(n) => format!("NoSource{n}"),
        Tag::NoPatch(n) => format!("NoPatch{n}"),
        Tag::Requires => "Requires".into(),
        Tag::BuildRequires => "BuildRequires".into(),
        Tag::Provides => "Provides".into(),
        Tag::Conflicts => "Conflicts".into(),
        Tag::Obsoletes => "Obsoletes".into(),
        Tag::Recommends => "Recommends".into(),
        Tag::Suggests => "Suggests".into(),
        Tag::Supplements => "Supplements".into(),
        Tag::Enhances => "Enhances".into(),
        Tag::BuildConflicts => "BuildConflicts".into(),
        Tag::OrderWithRequires => "OrderWithRequires".into(),
        Tag::BuildArch => "BuildArch".into(),
        Tag::ExclusiveArch => "ExclusiveArch".into(),
        Tag::ExcludeArch => "ExcludeArch".into(),
        Tag::ExclusiveOS => "ExclusiveOS".into(),
        Tag::ExcludeOS => "ExcludeOS".into(),
        Tag::BuildRoot => "BuildRoot".into(),
        Tag::Distribution => "Distribution".into(),
        Tag::Vendor => "Vendor".into(),
        Tag::Packager => "Packager".into(),
        Tag::AutoReq => "AutoReq".into(),
        Tag::AutoProv => "AutoProv".into(),
        Tag::AutoReqProv => "AutoReqProv".into(),
        Tag::Prefix => "Prefix".into(),
        Tag::Prefixes => "Prefixes".into(),
        Tag::BugURL => "BugURL".into(),
        Tag::ModularityLabel => "ModularityLabel".into(),
        Tag::VCS => "VCS".into(),
        Tag::Other(s) => s.clone(),
    }
}

fn qualifier_name(q: &TagQualifier) -> &str {
    match q {
        TagQualifier::Pre => "pre",
        TagQualifier::Post => "post",
        TagQualifier::Preun => "preun",
        TagQualifier::Postun => "postun",
        TagQualifier::Pretrans => "pretrans",
        TagQualifier::Posttrans => "posttrans",
        TagQualifier::Preuntrans => "preuntrans",
        TagQualifier::Postuntrans => "postuntrans",
        TagQualifier::Verify => "verify",
        TagQualifier::Interp => "interp",
        TagQualifier::Meta => "meta",
        TagQualifier::Other(s) => s,
    }
}

fn print_tag_value(p: &mut Printer<'_>, v: &TagValue) {
    match v {
        TagValue::Text(t) => print_text(p, t),
        TagValue::Dep(d) => print_dep_expr(p, d),
        TagValue::Bool(b) => p.raw(if *b { "1" } else { "0" }),
        TagValue::Number(n) => p.raw(&n.to_string()),
        TagValue::ArchList(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    p.raw_char(' ');
                }
                print_text(p, item);
            }
        }
    }
}

/// Used by `cond.rs` to detect empty texts; expose internal helper.
fn _empty_text_check(t: &Text) -> bool {
    t.segments.iter().all(|s| match s {
        TextSegment::Literal(l) => l.is_empty(),
        TextSegment::Macro(_) => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{DepAtom, DepExpr};
    use crate::printer::PrinterConfig;

    fn render(item: &PreambleItem<()>) -> String {
        let cfg = PrinterConfig::default();
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, &cfg);
        print_preamble_item(&mut p, item);
        buf
    }

    fn render_with(item: &PreambleItem<()>, cfg: &PrinterConfig) -> String {
        let mut buf = String::new();
        let mut p = Printer::new(&mut buf, cfg);
        print_preamble_item(&mut p, item);
        buf
    }

    #[test]
    fn name_aligned_col_16() {
        let item = PreambleItem {
            tag: Tag::Name,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Text(Text::from("hello")),
            data: (),
        };
        // "Name:" is 5 chars; padding to col 16 → 11 spaces.
        assert_eq!(render(&item), "Name:           hello\n");
    }

    #[test]
    fn long_tag_falls_back_to_single_space() {
        let item = PreambleItem {
            tag: Tag::OrderWithRequires,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Text(Text::from("x")),
            data: (),
        };
        // "OrderWithRequires:" = 18 chars > 16, so single space.
        assert_eq!(render(&item), "OrderWithRequires: x\n");
    }

    #[test]
    fn no_alignment_when_disabled() {
        let cfg = PrinterConfig::default().with_preamble_value_column(None);
        let item = PreambleItem {
            tag: Tag::Name,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Text(Text::from("hi")),
            data: (),
        };
        assert_eq!(render_with(&item, &cfg), "Name: hi\n");
    }

    #[test]
    fn numbered_source() {
        let item = PreambleItem {
            tag: Tag::Source(Some(0)),
            qualifiers: vec![],
            lang: None,
            value: TagValue::Text(Text::from("hello.tar.gz")),
            data: (),
        };
        assert!(render(&item).starts_with("Source0:"));
    }

    #[test]
    fn qualifier_list() {
        let item = PreambleItem {
            tag: Tag::Requires,
            qualifiers: vec![TagQualifier::Post, TagQualifier::Postun],
            lang: None,
            value: TagValue::Dep(DepExpr::Atom(DepAtom {
                name: Text::from("/bin/sh"),
                arch: None,
                constraint: None,
            })),
            data: (),
        };
        let out = render(&item);
        assert!(out.starts_with("Requires(post,postun):"));
        assert!(out.contains("/bin/sh"));
    }

    #[test]
    fn lang_qualifier() {
        let item = PreambleItem {
            tag: Tag::Summary,
            qualifiers: vec![],
            lang: Some("ru_RU.UTF-8".into()),
            value: TagValue::Text(Text::from("Привет")),
            data: (),
        };
        let out = render(&item);
        assert!(out.starts_with("Summary(ru_RU.UTF-8):"));
        assert!(out.contains("Привет"));
    }

    #[test]
    fn bool_tag() {
        let item = PreambleItem {
            tag: Tag::AutoReq,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Bool(false),
            data: (),
        };
        assert!(render(&item).ends_with(" 0\n"));
    }

    #[test]
    fn arch_list() {
        let item = PreambleItem {
            tag: Tag::BuildArch,
            qualifiers: vec![],
            lang: None,
            value: TagValue::ArchList(vec![Text::from("noarch")]),
            data: (),
        };
        assert!(render(&item).ends_with(" noarch\n"));
    }

    #[test]
    fn epoch_number() {
        let item = PreambleItem {
            tag: Tag::Epoch,
            qualifiers: vec![],
            lang: None,
            value: TagValue::Number(3),
            data: (),
        };
        assert!(render(&item).ends_with(" 3\n"));
    }
}
