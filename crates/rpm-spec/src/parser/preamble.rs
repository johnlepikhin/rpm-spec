//! Parser for preamble lines (`Tag: value`) and the content stream of
//! `%package` bodies.
//!
//! Public entry points:
//!
//! - [`parse_preamble_line`] consumes one `Tag[(...)] : value` line
//!   (with `\`-continuations). Returns `Vec<SpecItem>` because
//!   dep-bearing tags expand multi-dep lines into one
//!   [`PreambleItem`] per dependency.
//! - [`parse_preamble_content`] is the body-item parser used both by
//!   `%package` and by `%if` blocks nested inside `%package`. Returns
//!   `Vec<PreambleContent>` so the same multi-dep expansion applies.

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{
    PreambleContent, PreambleItem, Span, SpecItem, Tag, TagQualifier, TagValue, Text,
};
use crate::parse_result::codes;

use super::cond::parse_conditional;
use super::input::{Input, span_between};
use super::macros::parse_hash_comment;
use super::state::ParserState;
use super::text::parse_body_as_text;
use super::util::{blank_line, logical_line, space0};

// ---------------------------------------------------------------------
// Tag-name → Tag table & classifiers
// ---------------------------------------------------------------------

const TAG_TABLE: &[(&str, Tag)] = &[
    ("name", Tag::Name),
    ("version", Tag::Version),
    ("release", Tag::Release),
    ("summary", Tag::Summary),
    ("license", Tag::License),
    ("url", Tag::URL),
    ("group", Tag::Group),
    ("epoch", Tag::Epoch),
    ("icon", Tag::Icon),
    ("requires", Tag::Requires),
    ("buildrequires", Tag::BuildRequires),
    ("provides", Tag::Provides),
    ("conflicts", Tag::Conflicts),
    ("obsoletes", Tag::Obsoletes),
    ("recommends", Tag::Recommends),
    ("suggests", Tag::Suggests),
    ("supplements", Tag::Supplements),
    ("enhances", Tag::Enhances),
    ("buildconflicts", Tag::BuildConflicts),
    ("orderwithrequires", Tag::OrderWithRequires),
    ("buildarch", Tag::BuildArch),
    ("exclusivearch", Tag::ExclusiveArch),
    ("excludearch", Tag::ExcludeArch),
    ("exclusiveos", Tag::ExclusiveOS),
    ("excludeos", Tag::ExcludeOS),
    ("buildroot", Tag::BuildRoot),
    ("distribution", Tag::Distribution),
    ("vendor", Tag::Vendor),
    ("packager", Tag::Packager),
    ("autoreq", Tag::AutoReq),
    ("autoprov", Tag::AutoProv),
    ("autoreqprov", Tag::AutoReqProv),
    ("prefix", Tag::Prefix),
    ("prefixes", Tag::Prefixes),
    ("bugurl", Tag::BugURL),
    ("modularitylabel", Tag::ModularityLabel),
    ("vcs", Tag::VCS),
];

pub(crate) fn resolve_tag(name: &str) -> Tag {
    let lower = name.to_ascii_lowercase();

    // Numbered prefixes. Order: longer prefixes first (`nosource`
    // before `source`, `nopatch` before `patch`).
    if let Some(rest) = lower.strip_prefix("nosource") {
        if let Ok(n) = rest.parse::<u32>() {
            return Tag::NoSource(n);
        }
    }
    if let Some(rest) = lower.strip_prefix("nopatch") {
        if let Ok(n) = rest.parse::<u32>() {
            return Tag::NoPatch(n);
        }
    }
    if let Some(rest) = lower.strip_prefix("source") {
        if rest.is_empty() {
            return Tag::Source(None);
        }
        if let Ok(n) = rest.parse::<u32>() {
            return Tag::Source(Some(n));
        }
    }
    if let Some(rest) = lower.strip_prefix("patch") {
        if rest.is_empty() {
            return Tag::Patch(None);
        }
        if let Ok(n) = rest.parse::<u32>() {
            return Tag::Patch(Some(n));
        }
    }

    for (canonical, tag) in TAG_TABLE {
        if *canonical == lower {
            return tag.clone();
        }
    }

    Tag::Other(name.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Text,
    TextLang,
    Dep,
    Bool,
    Number,
    ArchList,
}

fn classify_tag_kind(tag: &Tag) -> TagKind {
    match tag {
        Tag::Source(_) | Tag::Patch(_) | Tag::NoSource(_) | Tag::NoPatch(_) => TagKind::Text,
        Tag::Summary | Tag::Group => TagKind::TextLang,
        Tag::Epoch => TagKind::Number,
        Tag::AutoReq | Tag::AutoProv | Tag::AutoReqProv => TagKind::Bool,
        Tag::BuildArch
        | Tag::ExclusiveArch
        | Tag::ExcludeArch
        | Tag::ExclusiveOS
        | Tag::ExcludeOS => TagKind::ArchList,
        Tag::Requires
        | Tag::BuildRequires
        | Tag::Provides
        | Tag::Conflicts
        | Tag::Obsoletes
        | Tag::Recommends
        | Tag::Suggests
        | Tag::Supplements
        | Tag::Enhances
        | Tag::BuildConflicts
        | Tag::OrderWithRequires => TagKind::Dep,
        _ => TagKind::Text,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParensExpect {
    Qualifiers,
    Lang,
    Auto,
}

fn classify_parens_for(tag: &Tag) -> ParensExpect {
    match tag {
        Tag::Requires
        | Tag::BuildRequires
        | Tag::Provides
        | Tag::Conflicts
        | Tag::Obsoletes
        | Tag::Recommends
        | Tag::Suggests
        | Tag::Supplements
        | Tag::Enhances
        | Tag::BuildConflicts
        | Tag::OrderWithRequires => ParensExpect::Qualifiers,
        Tag::Summary | Tag::Group => ParensExpect::Lang,
        _ => ParensExpect::Auto,
    }
}

// ---------------------------------------------------------------------
// Multi-dep splitter
// ---------------------------------------------------------------------

/// Split a multi-dep value into independent slices. Each returned slice
/// is one full dep (`foo`, `foo >= 1.0`, `(a and b)`, `/usr/bin/awk`).
pub fn split_dep_list(value: &str) -> Vec<&str> {
    let tokens = tokenize_dep_list(value);
    let mut deps: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let (start, mut end) = tokens[i];
        // Try to attach `op` + `version` from the next two tokens.
        if i + 1 < tokens.len() {
            let (op_s, op_e) = tokens[i + 1];
            if is_dep_operator(&value[op_s..op_e]) && i + 2 < tokens.len() {
                let (_, ver_e) = tokens[i + 2];
                end = ver_e;
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
        deps.push(&value[start..end]);
    }
    deps
}

fn is_dep_operator(s: &str) -> bool {
    matches!(s, "<" | ">" | "<=" | ">=" | "=" | "!=")
}

fn tokenize_dep_list(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b')' => {
                depth -= 1;
            }
            b' ' | b'\t' | b',' if depth == 0 => {
                if let Some(s_) = start.take() {
                    out.push((s_, i));
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s_) = start {
        out.push((s_, bytes.len()));
    }
    out
}

// ---------------------------------------------------------------------
// Qualifier / lang resolver
// ---------------------------------------------------------------------

enum QualsOrLang {
    Quals(Vec<TagQualifier>),
    Lang(String),
}

fn parse_quals_or_lang(inner: &str, expect: ParensExpect) -> QualsOrLang {
    let trimmed = inner.trim();
    match expect {
        ParensExpect::Qualifiers => QualsOrLang::Quals(parse_qualifier_list(trimmed)),
        ParensExpect::Lang => QualsOrLang::Lang(trimmed.to_owned()),
        ParensExpect::Auto => {
            // If every comma-separated token maps to a known qualifier
            // keyword, treat the whole as qualifiers. Otherwise lang.
            let parts: Vec<&str> = trimmed.split(',').map(str::trim).collect();
            if parts.iter().all(|p| try_parse_qualifier_keyword(p).is_some()) && !parts.is_empty() {
                QualsOrLang::Quals(parse_qualifier_list(trimmed))
            } else {
                QualsOrLang::Lang(trimmed.to_owned())
            }
        }
    }
}

fn parse_qualifier_list(inner: &str) -> Vec<TagQualifier> {
    inner
        .split(',')
        .map(|p| {
            let p = p.trim();
            try_parse_qualifier_keyword(p).unwrap_or_else(|| TagQualifier::Other(p.to_owned()))
        })
        .collect()
}

fn try_parse_qualifier_keyword(s: &str) -> Option<TagQualifier> {
    match s.to_ascii_lowercase().as_str() {
        "pre" => Some(TagQualifier::Pre),
        "post" => Some(TagQualifier::Post),
        "preun" => Some(TagQualifier::Preun),
        "postun" => Some(TagQualifier::Postun),
        "pretrans" => Some(TagQualifier::Pretrans),
        "posttrans" => Some(TagQualifier::Posttrans),
        "preuntrans" => Some(TagQualifier::Preuntrans),
        "postuntrans" => Some(TagQualifier::Postuntrans),
        "verify" => Some(TagQualifier::Verify),
        "interp" => Some(TagQualifier::Interp),
        "meta" => Some(TagQualifier::Meta),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Helpers: tag name and (...) extraction
// ---------------------------------------------------------------------

fn take_tag_name<'a>(input: Input<'a>) -> IResult<Input<'a>, &'a str> {
    let frag = *input.fragment();
    let mut iter = frag.char_indices();
    let Some((_, first)) = iter.next() else {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Alpha)));
    };
    if !first.is_ascii_alphabetic() {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Alpha)));
    }
    let mut end = first.len_utf8();
    for (i, c) in iter {
        if c.is_ascii_alphanumeric() || c == '_' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let (rest, _) = nom::Input::take_split(&input, end);
    Ok((rest, &frag[..end]))
}

fn take_balanced_parens<'a>(input: Input<'a>) -> IResult<Input<'a>, String> {
    let frag = *input.fragment();
    let bytes = frag.as_bytes();
    if !bytes.starts_with(b"(") {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }
    let mut depth: i32 = 0;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    match end {
        Some(e) => {
            let inner = frag[1..e - 1].to_owned();
            let (rest, _) = nom::Input::take_split(&input, e);
            Ok((rest, inner))
        }
        None => Err(nom::Err::Error(error_position!(input, ErrorKind::Tag))),
    }
}

// ---------------------------------------------------------------------
// Public entries
// ---------------------------------------------------------------------

/// Parse one preamble line. Returns multiple [`SpecItem::Preamble`]
/// entries when the source line packs several deps on the same tag.
pub fn parse_preamble_line<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Vec<SpecItem<Span>>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_name, name) = take_tag_name(after_ws)?;

    // Optional (...).
    let (after_quals, parens_inner) = if after_name.fragment().starts_with('(') {
        let (rest, inner) = take_balanced_parens(after_name)?;
        (rest, Some(inner))
    } else {
        (after_name, None)
    };

    // Allow optional space before the colon (rpm tolerates it).
    let (after_quals, _) = space0(after_quals)?;

    if !after_quals.fragment().starts_with(':') {
        return Err(nom::Err::Error(error_position!(after_quals, ErrorKind::Tag)));
    }
    let (after_colon, _) = nom::Input::take_split(&after_quals, 1);
    let (after_value_ws, _) = space0(after_colon)?;

    let (after_line, value_raw) = match logical_line(after_value_ws) {
        Ok(r) => r,
        Err(_) => (after_value_ws, String::new()),
    };
    let value_trim = value_raw.trim_end();

    let span = span_between(&start, &after_line);
    let tag = resolve_tag(name);

    let (qualifiers, lang) = match parens_inner {
        None => (Vec::new(), None),
        Some(inner) => match parse_quals_or_lang(&inner, classify_parens_for(&tag)) {
            QualsOrLang::Quals(qs) => (qs, None),
            QualsOrLang::Lang(l) => (Vec::new(), Some(l)),
        },
    };

    let items = build_preamble_items(state, tag, qualifiers, lang, value_trim, span);
    Ok((after_line, items))
}

fn build_preamble_items(
    state: &ParserState,
    tag: Tag,
    qualifiers: Vec<TagQualifier>,
    lang: Option<String>,
    value_trim: &str,
    span: Span,
) -> Vec<SpecItem<Span>> {
    let kind = classify_tag_kind(&tag);
    match kind {
        TagKind::Dep => {
            let slices = split_dep_list(value_trim);
            if slices.is_empty() {
                return vec![SpecItem::Preamble(PreambleItem {
                    tag,
                    qualifiers,
                    lang,
                    value: TagValue::Text(Text::new()),
                    data: span,
                })];
            }
            slices
                .iter()
                .filter_map(|slice| {
                    super::deps::parse_dep_expr(state, slice).ok().map(|dep| {
                        SpecItem::Preamble(PreambleItem {
                            tag: tag.clone(),
                            qualifiers: qualifiers.clone(),
                            lang: lang.clone(),
                            value: TagValue::Dep(dep),
                            data: span,
                        })
                    })
                })
                .collect()
        }
        TagKind::Bool => {
            let v = match value_trim.to_ascii_lowercase().as_str() {
                "0" | "no" | "false" => false,
                "1" | "yes" | "true" => true,
                _ => {
                    state.push_warning_code(
                        codes::W_INVALID_BOOL,
                        format!("expected boolean (0/1/yes/no) for tag, got `{value_trim}`"),
                        Some(span),
                    );
                    false
                }
            };
            vec![SpecItem::Preamble(PreambleItem {
                tag,
                qualifiers,
                lang,
                value: TagValue::Bool(v),
                data: span,
            })]
        }
        TagKind::Number => match value_trim.parse::<u32>() {
            Ok(n) => vec![SpecItem::Preamble(PreambleItem {
                tag,
                qualifiers,
                lang,
                value: TagValue::Number(n),
                data: span,
            })],
            Err(_) => {
                state.push_warning_code(
                    codes::W_INVALID_NUMBER,
                    format!("expected integer for tag, got `{value_trim}`"),
                    Some(span),
                );
                vec![SpecItem::Preamble(PreambleItem {
                    tag,
                    qualifiers,
                    lang,
                    value: TagValue::Text(parse_body_as_text(state, value_trim)),
                    data: span,
                })]
            }
        },
        TagKind::ArchList => {
            let arches: Vec<Text> = value_trim
                .split_whitespace()
                .map(|tok| parse_body_as_text(state, tok))
                .collect();
            vec![SpecItem::Preamble(PreambleItem {
                tag,
                qualifiers,
                lang,
                value: TagValue::ArchList(arches),
                data: span,
            })]
        }
        TagKind::Text | TagKind::TextLang => {
            vec![SpecItem::Preamble(PreambleItem {
                tag,
                qualifiers,
                lang,
                value: TagValue::Text(parse_body_as_text(state, value_trim)),
                data: span,
            })]
        }
    }
}

/// Parse one item of a `%package` body. Returns `Vec` so multi-dep
/// preamble lines expand into multiple [`PreambleContent::Item`]s.
pub fn parse_preamble_content<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Vec<PreambleContent<Span>>> {
    // Blank line.
    if let Ok((rest, _)) = blank_line(input) {
        if rest.location_offset() > input.location_offset() {
            return Ok((rest, vec![PreambleContent::Blank]));
        }
    }
    // # comment.
    let after_ws = match space0(input) {
        Ok((r, _)) => r,
        Err(_) => input,
    };
    if after_ws.fragment().starts_with('#') {
        if let Ok((rest, SpecItem::Comment(c))) = parse_hash_comment(state, input) {
            return Ok((rest, vec![PreambleContent::Comment(c)]));
        }
    }
    // %if / %ifarch / %ifos block.
    if let Ok((rest, cond)) = parse_conditional(state, input, parse_preamble_content) {
        return Ok((rest, vec![PreambleContent::Conditional(cond)]));
    }
    // Preamble line.
    let (rest, items) = parse_preamble_line(state, input)?;
    let mapped: Vec<PreambleContent<Span>> = items
        .into_iter()
        .filter_map(|item| match item {
            SpecItem::Preamble(pi) => Some(PreambleContent::Item(pi)),
            _ => None,
        })
        .collect();
    if mapped.is_empty() {
        return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
    }
    Ok((rest, mapped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{DepExpr, TextSegment};

    fn parse_line(src: &str) -> (Vec<SpecItem<Span>>, ParserState) {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, items) = parse_preamble_line(&state, inp).unwrap();
        (items, state)
    }

    fn as_preamble(item: &SpecItem<Span>) -> &PreambleItem<Span> {
        match item {
            SpecItem::Preamble(p) => p,
            other => panic!("expected Preamble, got {other:?}"),
        }
    }

    #[test]
    fn resolve_tag_basic() {
        assert!(matches!(resolve_tag("Name"), Tag::Name));
        assert!(matches!(resolve_tag("VERSION"), Tag::Version));
        assert!(matches!(resolve_tag("BuildRequires"), Tag::BuildRequires));
    }

    #[test]
    fn resolve_tag_numbered() {
        assert_eq!(resolve_tag("Source"), Tag::Source(None));
        assert_eq!(resolve_tag("Source0"), Tag::Source(Some(0)));
        assert_eq!(resolve_tag("Source42"), Tag::Source(Some(42)));
        assert_eq!(resolve_tag("Patch"), Tag::Patch(None));
        assert_eq!(resolve_tag("Patch5"), Tag::Patch(Some(5)));
        assert_eq!(resolve_tag("NoSource0"), Tag::NoSource(0));
        assert_eq!(resolve_tag("NoPatch1"), Tag::NoPatch(1));
    }

    #[test]
    fn resolve_tag_unknown_other() {
        match resolve_tag("XCustomTag") {
            Tag::Other(s) => assert_eq!(s, "XCustomTag"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn parse_simple_text_tag() {
        let (items, _) = parse_line("Name: hello\n");
        assert_eq!(items.len(), 1);
        let p = as_preamble(&items[0]);
        assert!(matches!(p.tag, Tag::Name));
        match &p.value {
            TagValue::Text(t) => assert_eq!(t.literal_str(), Some("hello")),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_value_with_macros() {
        let (items, _) = parse_line("Release: 1%{?dist}\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::Text(t) => {
                assert_eq!(t.segments.len(), 2);
                assert!(matches!(&t.segments[0], TextSegment::Literal(s) if s == "1"));
                assert!(matches!(&t.segments[1], TextSegment::Macro(m) if m.name == "dist"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_source_numbered() {
        let (items, _) = parse_line("Source0: hello-%{version}.tar.gz\n");
        let p = as_preamble(&items[0]);
        assert_eq!(p.tag, Tag::Source(Some(0)));
    }

    #[test]
    fn parse_epoch_numeric() {
        let (items, _) = parse_line("Epoch: 3\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::Number(n) => assert_eq!(*n, 3),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_bool_tag_yes() {
        let (items, _) = parse_line("AutoReq: yes\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::Bool(true) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_bool_tag_zero() {
        let (items, _) = parse_line("AutoProv: 0\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::Bool(false) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_archlist() {
        let (items, _) = parse_line("BuildArch: noarch\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::ArchList(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].literal_str(), Some("noarch"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_archlist_multiple() {
        let (items, _) = parse_line("ExclusiveArch: x86_64 aarch64 ppc64le\n");
        let p = as_preamble(&items[0]);
        match &p.value {
            TagValue::ArchList(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[2].literal_str(), Some("ppc64le"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_qualifier_list() {
        let (items, _) = parse_line("Requires(post,postun): /bin/sh\n");
        let p = as_preamble(&items[0]);
        assert_eq!(
            p.qualifiers,
            vec![TagQualifier::Post, TagQualifier::Postun]
        );
        match &p.value {
            TagValue::Dep(DepExpr::Atom(a)) => {
                assert_eq!(a.name.literal_str(), Some("/bin/sh"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_lang_qualifier() {
        let (items, _) = parse_line("Summary(ru_RU.UTF-8): Greeter\n");
        let p = as_preamble(&items[0]);
        assert!(p.qualifiers.is_empty());
        assert_eq!(p.lang.as_deref(), Some("ru_RU.UTF-8"));
    }

    #[test]
    fn parse_multi_dep_whitespace() {
        let (items, _) = parse_line("Requires: foo bar baz\n");
        assert_eq!(items.len(), 3);
        for it in &items {
            let p = as_preamble(it);
            assert!(matches!(p.tag, Tag::Requires));
            assert!(matches!(p.value, TagValue::Dep(_)));
        }
    }

    #[test]
    fn parse_multi_dep_comma_and_op() {
        let (items, _) = parse_line("Requires: foo, bar >= 1.0 baz\n");
        assert_eq!(items.len(), 3);
        let names: Vec<&str> = items
            .iter()
            .map(|it| match &as_preamble(it).value {
                TagValue::Dep(DepExpr::Atom(a)) => a.name.literal_str().unwrap(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(names, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn parse_multi_dep_rich() {
        let (items, _) = parse_line("Requires: (foo and bar) baz\n");
        assert_eq!(items.len(), 2);
        match &as_preamble(&items[0]).value {
            TagValue::Dep(DepExpr::Rich(_)) => {}
            _ => panic!("first should be rich"),
        }
        match &as_preamble(&items[1]).value {
            TagValue::Dep(DepExpr::Atom(a)) => {
                assert_eq!(a.name.literal_str(), Some("baz"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_multiline_via_backslash() {
        let (items, _) =
            parse_line("Description: long \\\nand longer \\\nand longest\n");
        let p = as_preamble(&items[0]);
        let text = match &p.value {
            TagValue::Text(t) => t,
            _ => panic!(),
        };
        // logical_line joins with `\n` between continued segments,
        // backslashes stripped.
        assert_eq!(text.literal_str(), Some("long\nand longer\nand longest"));
    }
}
