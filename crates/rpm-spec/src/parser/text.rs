//! Parser for [`Text`] (literal + macro segments) and the full [`MacroRef`]
//! grammar.
//!
//! Public entry points:
//!
//! - [`parse_text`] consumes characters until a caller-supplied terminator
//!   is reached, splitting them into [`TextSegment::Literal`] runs and
//!   [`TextSegment::Macro`] nodes.
//! - [`parse_macro_ref`] starts at a `%` and parses a single macro
//!   reference, handling every surface form (`%foo`, `%{foo}`, `%{?foo}`,
//!   `%{!?foo:VALUE}`, `%{foo arg1 arg2}`, `%(shell)`, `%[expr]`,
//!   `%{lua:…}`, `%{shrink:…}`, etc.).
//!
//! The body of every nested macro is parsed recursively as [`Text`]: macros
//! inside `%(echo %{name})` or `%{shrink:%{foo}}` become structured nodes,
//! not flat strings. This keeps the AST invariant that *every* macro use
//! site is a [`MacroRef`] node — never lost in a string blob.

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{BuiltinMacro, ConditionalMacro, MacroKind, MacroRef, Text, TextSegment};
use crate::parse_result::codes;

use super::input::Input;
use super::state::ParserState;

/// Builtin macro keywords recognized by RPM. Lookup is case-sensitive and
/// verbatim: an unknown body before the `:` falls into
/// [`BuiltinMacro::Other`].
const BUILTIN_KEYWORDS: &[(&str, BuiltinMacro)] = &[
    ("expand", BuiltinMacro::Expand),
    ("shrink", BuiltinMacro::Shrink),
    ("quote", BuiltinMacro::Quote),
    ("gsub", BuiltinMacro::Gsub),
    ("sub", BuiltinMacro::Sub),
    ("len", BuiltinMacro::Len),
    ("upper", BuiltinMacro::Upper),
    ("lower", BuiltinMacro::Lower),
    ("reverse", BuiltinMacro::Reverse),
    ("basename", BuiltinMacro::Basename),
    ("dirname", BuiltinMacro::Dirname),
    ("suffix", BuiltinMacro::Suffix),
    ("exists", BuiltinMacro::Exists),
    ("load", BuiltinMacro::Load),
    ("echo", BuiltinMacro::Echo),
    ("warn", BuiltinMacro::Warn),
    ("error", BuiltinMacro::Error),
    ("dnl", BuiltinMacro::Dnl),
    ("trace", BuiltinMacro::Trace),
    ("dump", BuiltinMacro::Dump),
];

fn builtin_lookup(name: &str) -> Option<BuiltinMacro> {
    BUILTIN_KEYWORDS
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.clone())
}

/// Parse [`Text`] until the caller-supplied terminator returns `true` for
/// the *next* unconsumed character. The terminator is also the stop
/// condition at end-of-input.
pub fn parse_text<'a>(
    state: &ParserState,
    mut input: Input<'a>,
    is_terminator: &impl Fn(char) -> bool,
) -> IResult<Input<'a>, Text> {
    let mut segments: Vec<TextSegment> = Vec::new();
    let mut literal = String::new();

    loop {
        let frag = *input.fragment();
        let mut chars = frag.chars();
        let next = chars.next();

        let stop = match next {
            None => true,
            Some(c) => is_terminator(c) && c != '%',
        };
        if stop {
            break;
        }

        // SAFETY of unwrap: `next` is `Some` because we did not break.
        let c = next.unwrap();

        if c == '%' {
            // `%%` decodes to a single literal '%'.
            if frag.starts_with("%%") {
                literal.push('%');
                input = advance_bytes(input, 2);
                continue;
            }
            // Try macro; fall back to literal '%' on parse failure.
            match parse_macro_ref(state, input) {
                Ok((rest, m)) => {
                    flush_literal(&mut literal, &mut segments);
                    segments.push(TextSegment::Macro(Box::new(m)));
                    input = rest;
                    continue;
                }
                Err(_) => {
                    // Could not parse a macro. Consume one byte and emit a
                    // warning, since '%' as a literal in source is normally
                    // written as '%%'.
                    literal.push('%');
                    input = advance_one_char(input);
                    state.push_warning_code(
                        codes::W_STRAY_PERCENT,
                        "stray '%' in text; if this is literal, use '%%'",
                        Some(super::input::span_at(&input)),
                    );
                    continue;
                }
            }
        } else {
            // Take literal run until next '%' or terminator.
            let mut taken = 0usize;
            for ch in frag.chars() {
                if ch == '%' || is_terminator(ch) {
                    break;
                }
                taken += ch.len_utf8();
            }
            if taken == 0 {
                break;
            }
            literal.push_str(&frag[..taken]);
            // Advance input by `taken` bytes.
            input = advance_bytes(input, taken);
        }
    }

    flush_literal(&mut literal, &mut segments);
    Ok((input, Text { segments }))
}

/// Parse a raw `&str` body (e.g. a preamble value, a macro definition's
/// body, a dep-atom's name) into a [`Text`] with macros split out into
/// [`TextSegment::Macro`] nodes.
///
/// On macro grammar failure, the whole body is preserved as a single
/// literal — recovery is silent because [`parse_text`] already emits
/// per-character warnings via the shared [`ParserState`].
pub fn parse_body_as_text(state: &ParserState, raw: &str) -> Text {
    if raw.is_empty() {
        return Text::new();
    }
    let inp = Input::new(raw);
    match parse_text(state, inp, &|_c| false) {
        Ok((_rest, text)) => text,
        Err(_) => Text { segments: vec![TextSegment::Literal(raw.to_owned())] },
    }
}

fn flush_literal(buf: &mut String, segments: &mut Vec<TextSegment>) {
    if !buf.is_empty() {
        segments.push(TextSegment::Literal(std::mem::take(buf)));
    }
}

fn advance_one_char(input: Input<'_>) -> Input<'_> {
    let frag = *input.fragment();
    let n = frag.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
    advance_bytes(input, n)
}

fn advance_bytes(input: Input<'_>, n: usize) -> Input<'_> {
    // `take_split(n)` returns `(remaining, taken)` — the first element is
    // the input *after* the first `n` items.
    let (rest, _taken) = input.take_split(n);
    rest
}

// nom's `LocatedSpan::take_split` requires the `Input` trait; bring it in.
use nom::Input as _;

/// Parse a single [`MacroRef`] starting at the current `%` cursor.
pub fn parse_macro_ref<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, MacroRef> {
    let frag = *input.fragment();
    let mut chars = frag.char_indices();

    // Consume the leading '%'.
    let (_, percent) = chars.next().ok_or_else(|| nom_err(input, ErrorKind::Tag))?;
    if percent != '%' {
        return Err(nom_err(input, ErrorKind::Tag));
    }
    let after_percent = advance_bytes(input, 1);
    let next_frag = *after_percent.fragment();

    // %% — literal '%'. We treat this as a parse failure for macro_ref so
    // callers (which usually want a *macro*) can fall back to literal.
    if next_frag.starts_with('%') {
        return Err(nom_err(input, ErrorKind::Tag));
    }

    // %(shell)
    if next_frag.starts_with('(') {
        return parse_shell_macro(state, input);
    }
    // %[expr]
    if next_frag.starts_with('[') {
        return parse_bracketed_expr_macro(state, input);
    }
    // %{...}
    if next_frag.starts_with('{') {
        return parse_braced_macro(state, input);
    }

    // Plain forms: %foo, %1, %*, %**, %#, %-f (rare bare flag), %?foo,
    // %!?foo. We allow `%?foo` / `%!?foo` as conditional plain forms; the
    // common spelling uses braces, but tolerating bare is harmless.
    parse_plain_macro(state, input)
}

fn parse_plain_macro<'a>(
    _state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, MacroRef> {
    // We're positioned at '%' already; skip it.
    let after_percent = advance_bytes(input, 1);
    let frag = *after_percent.fragment();

    // Conditional prefix.
    let (conditional, after_prefix) = if frag.starts_with("!?") {
        (ConditionalMacro::IfNotDefined, advance_bytes(after_percent, 2))
    } else if frag.starts_with('?') {
        (ConditionalMacro::IfDefined, advance_bytes(after_percent, 1))
    } else {
        (ConditionalMacro::None, after_percent)
    };

    let frag2 = *after_prefix.fragment();
    let (name, name_len) = read_plain_name(frag2);
    if name.is_empty() {
        return Err(nom_err(input, ErrorKind::AlphaNumeric));
    }
    let rest = advance_bytes(after_prefix, name_len);

    Ok((
        rest,
        MacroRef {
            kind:        MacroKind::Plain,
            name:        name.to_owned(),
            args:        Vec::new(),
            conditional,
            with_value:  None,
        },
    ))
}

/// Read a plain-form name from the start of `s`. Plain names cover
/// `[A-Za-z_][A-Za-z0-9_]*`, all-digit positional (`%1`, `%42`), the
/// arity sigils `*`, `**`, `#`, and `-flag` / `-flag*` forms.
fn read_plain_name(s: &str) -> (&str, usize) {
    let mut iter = s.char_indices();
    let Some((_, first)) = iter.next() else {
        return ("", 0);
    };

    if first == '*' {
        // `*` or `**`
        if s.starts_with("**") {
            return ("**", 2);
        }
        return ("*", 1);
    }
    if first == '#' {
        return ("#", 1);
    }
    if first == '-' {
        // `-flag` optionally `-flag*`
        let mut end = 1;
        for (i, c) in s[1..].char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end = 1 + i + c.len_utf8();
            } else {
                break;
            }
        }
        if end == 1 {
            return ("", 0);
        }
        if s[end..].starts_with('*') {
            return (&s[..end + 1], end + 1);
        }
        return (&s[..end], end);
    }
    if first.is_ascii_digit() {
        let mut end = 0;
        for (i, c) in s.char_indices() {
            if c.is_ascii_digit() {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }
        return (&s[..end], end);
    }
    if first.is_ascii_alphabetic() || first == '_' {
        let mut end = 0;
        for (i, c) in s.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }
        return (&s[..end], end);
    }
    ("", 0)
}

fn parse_shell_macro<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, MacroRef> {
    // Consume '%('.
    let inside = advance_bytes(input, 2);
    let (after_inside, body) =
        parse_text(state, inside, &|c| c == ')')?;
    let after_inside_frag = *after_inside.fragment();
    if !after_inside_frag.starts_with(')') {
        state.push_warning_code(
            codes::W_UNTERMINATED_MACRO,
            "unterminated %(...) macro: expected ')'",
            Some(super::input::span_at(&after_inside)),
        );
        return Ok((
            after_inside,
            MacroRef {
                kind:        MacroKind::Shell,
                name:        String::new(),
                args:        vec![body],
                conditional: ConditionalMacro::None,
                with_value:  None,
            },
        ));
    }
    let rest = advance_bytes(after_inside, 1);
    Ok((
        rest,
        MacroRef {
            kind:        MacroKind::Shell,
            name:        String::new(),
            args:        vec![body],
            conditional: ConditionalMacro::None,
            with_value:  None,
        },
    ))
}

fn parse_bracketed_expr_macro<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, MacroRef> {
    // Consume '%['.
    let inside = advance_bytes(input, 2);
    let (after_inside, body) =
        parse_text(state, inside, &|c| c == ']')?;
    let after_inside_frag = *after_inside.fragment();
    if !after_inside_frag.starts_with(']') {
        state.push_warning_code(
            codes::W_UNTERMINATED_MACRO,
            "unterminated %[...] expression: expected ']'",
            Some(super::input::span_at(&after_inside)),
        );
        return Ok((
            after_inside,
            MacroRef {
                kind:        MacroKind::Expr,
                name:        String::new(),
                args:        vec![body],
                conditional: ConditionalMacro::None,
                with_value:  None,
            },
        ));
    }
    let rest = advance_bytes(after_inside, 1);
    Ok((
        rest,
        MacroRef {
            kind:        MacroKind::Expr,
            name:        String::new(),
            args:        vec![body],
            conditional: ConditionalMacro::None,
            with_value:  None,
        },
    ))
}

fn parse_braced_macro<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, MacroRef> {
    // Consume '%{'.
    let mut cursor = advance_bytes(input, 2);
    let frag = *cursor.fragment();

    // Detect conditional prefix.
    let (conditional, frag_after_prefix, prefix_len) = if let Some(rest) = frag.strip_prefix("!?") {
        (ConditionalMacro::IfNotDefined, rest, 2)
    } else if let Some(rest) = frag.strip_prefix('?') {
        (ConditionalMacro::IfDefined, rest, 1)
    } else {
        (ConditionalMacro::None, frag, 0)
    };
    cursor = advance_bytes(cursor, prefix_len);

    // Read name.
    let (name, name_len) = read_plain_name(frag_after_prefix);
    if name.is_empty() {
        // Empty braces or malformed: consume to matching '}' as a single
        // text node with a warning, so we don't get stuck.
        state.push_warning_code(
            codes::W_MACRO_EMPTY_NAME,
            "macro reference with empty or invalid name",
            Some(super::input::span_at(&cursor)),
        );
    }
    cursor = advance_bytes(cursor, name_len);

    // Look at the byte right after the name to decide kind.
    let after_name = *cursor.fragment();
    let mut kind = MacroKind::Braced;
    let mut args: Vec<Text> = Vec::new();
    let mut with_value: Option<Text> = None;

    let known_builtin = builtin_lookup(name);

    if after_name.starts_with(':') {
        // Two cases: conditional with_value, or builtin body, or
        // builtin-like for unknown keyword.
        let after_colon = advance_bytes(cursor, 1);
        let (after_body, body) =
            parse_text(state, after_colon, &|c| c == '}')?;
        match conditional {
            ConditionalMacro::None => {
                // builtin or unknown :body.
                if let Some(b) = known_builtin {
                    kind = MacroKind::Builtin(b);
                } else if name == "lua" {
                    kind = MacroKind::Lua;
                } else if name == "expr" {
                    kind = MacroKind::Expr;
                } else {
                    kind = MacroKind::Builtin(BuiltinMacro::Other(name.into()));
                }
                args.push(body);
            }
            ConditionalMacro::IfDefined | ConditionalMacro::IfNotDefined => {
                with_value = Some(body);
            }
        }
        cursor = after_body;
    } else if let Some(rest_after_ws) = strip_inline_space(after_name) {
        // Parametric macro: %{name arg1 arg2 ...}
        kind = MacroKind::Parametric;
        let ws_len = after_name.len() - rest_after_ws.len();
        cursor = advance_bytes(cursor, ws_len);

        // Parse args: each is a Text terminated by whitespace or `}`.
        loop {
            let frag = *cursor.fragment();
            if frag.starts_with('}') || frag.is_empty() {
                break;
            }
            let (after_arg, arg) =
                parse_text(state, cursor, &|c| c == '}' || c == ' ' || c == '\t')?;
            if arg.segments.is_empty() {
                break;
            }
            args.push(arg);
            cursor = after_arg;
            // Skip whitespace between args.
            let frag2 = *cursor.fragment();
            if let Some(rest_ws) = strip_inline_space(frag2) {
                let ws_len2 = frag2.len() - rest_ws.len();
                cursor = advance_bytes(cursor, ws_len2);
            }
        }
    } else if known_builtin.is_some() && !after_name.starts_with('}') {
        // Builtin without body — odd, treat as plain braced and warn.
        state.push_warning_code(
            codes::W_BUILTIN_MISSING_BODY,
            "builtin macro reference missing its ':' body",
            Some(super::input::span_at(&cursor)),
        );
        kind = MacroKind::Builtin(known_builtin.unwrap());
    }

    // Expect closing '}'.
    let frag2 = *cursor.fragment();
    if frag2.starts_with('}') {
        cursor = advance_bytes(cursor, 1);
    } else {
        state.push_warning_code(
            codes::W_UNTERMINATED_MACRO,
            "unterminated %{...} macro: expected '}'",
            Some(super::input::span_at(&cursor)),
        );
    }

    Ok((
        cursor,
        MacroRef { kind, name: name.to_owned(), args, conditional, with_value },
    ))
}

fn strip_inline_space(s: &str) -> Option<&str> {
    let mut iter = s.char_indices();
    let (_, first) = iter.next()?;
    if first == ' ' || first == '\t' {
        let mut end = first.len_utf8();
        for (_, c) in iter {
            if c == ' ' || c == '\t' {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        Some(&s[end..])
    } else {
        None
    }
}

fn nom_err(input: Input<'_>, kind: ErrorKind) -> nom::Err<nom::error::Error<Input<'_>>> {
    nom::Err::Error(error_position!(input, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BuiltinMacro, ConditionalMacro, MacroKind};

    fn parse_one(src: &str) -> (Text, ParserState) {
        let state = ParserState::new();
        let input = Input::new(src);
        let (_rest, t) = parse_text(&state, input, &|c| c == '\n').unwrap();
        (t, state)
    }

    fn first_macro(t: &Text) -> &MacroRef {
        match t.segments.first().expect("at least one segment") {
            TextSegment::Macro(m) => m,
            _ => panic!("expected macro segment, got {:?}", t.segments[0]),
        }
    }

    #[test]
    fn plain_text_only() {
        let (t, _s) = parse_one("hello world");
        assert_eq!(t.literal_str(), Some("hello world"));
    }

    #[test]
    fn plain_macro() {
        let (t, _s) = parse_one("%name");
        let m = first_macro(&t);
        assert_eq!(m.name, "name");
        assert_eq!(m.kind, MacroKind::Plain);
        assert_eq!(m.conditional, ConditionalMacro::None);
    }

    #[test]
    fn double_percent_decodes_to_literal() {
        let (t, _s) = parse_one("100%% sure");
        assert_eq!(t.segments.len(), 1);
        match &t.segments[0] {
            TextSegment::Literal(l) => assert_eq!(l, "100% sure"),
            _ => panic!("expected literal"),
        }
    }

    #[test]
    fn braced_macro() {
        let (t, _s) = parse_one("%{name}");
        let m = first_macro(&t);
        assert_eq!(m.name, "name");
        assert_eq!(m.kind, MacroKind::Braced);
    }

    #[test]
    fn conditional_braced() {
        let (t, _s) = parse_one("%{?dist}");
        let m = first_macro(&t);
        assert_eq!(m.name, "dist");
        assert_eq!(m.conditional, ConditionalMacro::IfDefined);
    }

    #[test]
    fn conditional_not_defined() {
        let (t, _s) = parse_one("%{!?with_x:default}");
        let m = first_macro(&t);
        assert_eq!(m.name, "with_x");
        assert_eq!(m.conditional, ConditionalMacro::IfNotDefined);
        assert_eq!(m.with_value.as_ref().unwrap().literal_str(), Some("default"));
    }

    #[test]
    fn conditional_with_value_macro_inside() {
        let (t, _s) = parse_one("%{?foo:%{bar}}");
        let m = first_macro(&t);
        let with = m.with_value.as_ref().unwrap();
        assert_eq!(with.segments.len(), 1);
        match &with.segments[0] {
            TextSegment::Macro(inner) => {
                assert_eq!(inner.name, "bar");
                assert_eq!(inner.kind, MacroKind::Braced);
            }
            _ => panic!("expected nested macro"),
        }
    }

    #[test]
    fn parametric_macro() {
        let (t, _s) = parse_one("%{foo a b}");
        let m = first_macro(&t);
        assert_eq!(m.name, "foo");
        assert_eq!(m.kind, MacroKind::Parametric);
        assert_eq!(m.args.len(), 2);
        assert_eq!(m.args[0].literal_str(), Some("a"));
        assert_eq!(m.args[1].literal_str(), Some("b"));
    }

    #[test]
    fn shell_macro() {
        let (t, _s) = parse_one("%(date +%%Y)");
        let m = first_macro(&t);
        assert_eq!(m.kind, MacroKind::Shell);
        // `%%` inside shell body decodes to literal '%'.
        assert_eq!(m.args[0].literal_str(), Some("date +%Y"));
    }

    #[test]
    fn expr_brackets_macro() {
        let (t, _s) = parse_one("%[1+1]");
        let m = first_macro(&t);
        assert_eq!(m.kind, MacroKind::Expr);
        assert_eq!(m.args[0].literal_str(), Some("1+1"));
    }

    #[test]
    fn builtin_shrink() {
        let (t, _s) = parse_one("%{shrink:  a   b  }");
        let m = first_macro(&t);
        assert!(matches!(m.kind, MacroKind::Builtin(BuiltinMacro::Shrink)));
        assert_eq!(m.args[0].literal_str(), Some("  a   b  "));
    }

    #[test]
    fn lua_block() {
        let (t, _s) = parse_one("%{lua: print('hi') }");
        let m = first_macro(&t);
        assert_eq!(m.kind, MacroKind::Lua);
    }

    #[test]
    fn expr_keyword() {
        let (t, _s) = parse_one("%{expr:1+1}");
        let m = first_macro(&t);
        assert_eq!(m.kind, MacroKind::Expr);
    }

    #[test]
    fn unknown_builtin_is_other() {
        let (t, _s) = parse_one("%{frobnicate:body}");
        let m = first_macro(&t);
        match &m.kind {
            MacroKind::Builtin(BuiltinMacro::Other(name)) => {
                assert_eq!(name.as_ref(), "frobnicate");
            }
            other => panic!("expected Builtin::Other, got {other:?}"),
        }
    }

    #[test]
    fn positional() {
        let (t, _s) = parse_one("%1");
        let m = first_macro(&t);
        assert_eq!(m.name, "1");
        assert_eq!(m.kind, MacroKind::Plain);
        assert_eq!(m.positional_index(), Some(1));
    }

    #[test]
    fn star_args() {
        let (t1, _) = parse_one("%*");
        assert_eq!(first_macro(&t1).name, "*");
        let (t2, _) = parse_one("%**");
        assert_eq!(first_macro(&t2).name, "**");
    }

    #[test]
    fn arg_count() {
        let (t, _s) = parse_one("%#");
        assert_eq!(first_macro(&t).name, "#");
    }

    #[test]
    fn flag_ref_braced() {
        let (t, _s) = parse_one("%{-f}");
        let m = first_macro(&t);
        assert_eq!(m.name, "-f");
        assert_eq!(m.flag_ref(), Some(("f", false)));

        let (t2, _) = parse_one("%{-f*}");
        let m2 = first_macro(&t2);
        assert_eq!(m2.name, "-f*");
        assert_eq!(m2.flag_ref(), Some(("f", true)));
    }

    #[test]
    fn nested_in_shell() {
        let (t, _s) = parse_one("%(echo %{name})");
        let m = first_macro(&t);
        assert_eq!(m.kind, MacroKind::Shell);
        let body = &m.args[0];
        let mut iter = body.segments.iter();
        assert!(matches!(iter.next(), Some(TextSegment::Literal(s)) if s == "echo "));
        match iter.next() {
            Some(TextSegment::Macro(inner)) => {
                assert_eq!(inner.name, "name");
                assert_eq!(inner.kind, MacroKind::Braced);
            }
            other => panic!("expected nested macro, got {other:?}"),
        }
    }

    #[test]
    fn mixed_literal_and_macros() {
        let (t, _s) = parse_one("prefix-%{name}-%{version}");
        assert_eq!(t.segments.len(), 4);
        assert!(matches!(&t.segments[0], TextSegment::Literal(s) if s == "prefix-"));
        assert!(matches!(&t.segments[1], TextSegment::Macro(m) if m.name == "name"));
        assert!(matches!(&t.segments[2], TextSegment::Literal(s) if s == "-"));
        assert!(matches!(&t.segments[3], TextSegment::Macro(m) if m.name == "version"));
    }

    #[test]
    fn stray_percent_warns_and_keeps_literal() {
        let (t, state) = parse_one("50% off");
        assert_eq!(t.literal_str(), Some("50% off"));
        assert!(!state.snapshot_diagnostics().is_empty());
    }
}
