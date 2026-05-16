//! Parser for one dependency expression: a classic atom (name + optional
//! arch + optional version constraint), a file dependency, or a rich /
//! boolean dependency tree (`(A and B)`, `(then if cond else otherwise)`,
//! …).
//!
//! Input is a string slice produced by the multi-dep splitter in
//! `parser/preamble.rs`. Spans are not attached to inner nodes (atoms,
//! EVRs); the enclosing [`crate::ast::PreambleItem`] keeps a span that
//! covers the whole source line.

use crate::ast::{BoolDep, DepAtom, DepConstraint, DepExpr, EVR, VerOp};
use crate::parse_result::codes;

use super::state::ParserState;
use super::text::parse_body_as_text;

/// Parse one dep slice into a [`DepExpr`].
///
/// On unrecoverable shape errors a [`crate::parse_result::Diagnostic`]
/// is pushed into the [`ParserState`] and `Err(())` is returned; callers
/// in `parser/preamble.rs` drop the failed slice and continue. The unit
/// error is intentional — the actual error information lives in the
/// diagnostic stream.
#[allow(clippy::result_unit_err)]
pub fn parse_dep_expr(state: &ParserState, slice: &str) -> Result<DepExpr, ()> {
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        state.push_warning_code(codes::W_EMPTY_DEP, "empty dependency in dep list", None);
        return Err(());
    }
    if trimmed.starts_with('(') {
        return parse_rich_top(state, trimmed);
    }
    Ok(DepExpr::Atom(parse_atom(state, trimmed)?))
}

// ---------------------------------------------------------------------
// Atom parsing
// ---------------------------------------------------------------------

fn parse_atom(state: &ParserState, slice: &str) -> Result<DepAtom, ()> {
    let s = slice.trim();
    if s.is_empty() {
        return Err(());
    }

    let (name_part, constraint) = match find_constraint_operator(s) {
        Some((op_start, op_end, op)) => {
            let evr_str = s[op_end..].trim();
            let evr = parse_evr(state, evr_str);
            (s[..op_start].trim_end(), Some(DepConstraint { op, evr }))
        }
        None => (s, None),
    };

    if name_part.is_empty() {
        state.push_error_code(
            codes::E_DEP_ATOM_NO_NAME,
            "dependency atom has no name",
            None,
        );
        return Err(());
    }

    let (name_str, arch_str) = split_arch(name_part);
    let name = parse_body_as_text(state, name_str);
    let arch = arch_str.map(|a| parse_body_as_text(state, a));

    Ok(DepAtom {
        name,
        arch,
        constraint,
    })
}

/// Find the first version-comparison operator at paren-depth 0.
///
/// Returns `(start_byte, end_byte, VerOp)`. Order matters: 2-char
/// operators must be checked before their 1-char prefixes.
fn find_constraint_operator(s: &str) -> Option<(usize, usize, VerOp)> {
    const OPS: &[(&str, VerOp)] = &[
        ("<=", VerOp::Le),
        (">=", VerOp::Ge),
        ("!=", VerOp::Ne),
        ("<", VerOp::Lt),
        (">", VerOp::Gt),
        ("=", VerOp::Eq),
    ];
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0 => {
                let rest = &s[i..];
                let mut matched = None;
                for (kw, op) in OPS {
                    if rest.starts_with(kw) {
                        matched = Some((i, i + kw.len(), *op));
                        break;
                    }
                }
                if let Some(m) = matched {
                    return Some(m);
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// Apply the arch heuristic: if the trailing `(X)` of the name contains
/// only `[A-Za-z0-9_-]` with no `.` and no nested `(`, treat `X` as the
/// architecture qualifier. Otherwise leave the name untouched.
fn split_arch(name: &str) -> (&str, Option<&str>) {
    if !name.ends_with(')') {
        return (name, None);
    }
    let bytes = name.as_bytes();
    let mut depth: i32 = 0;
    let mut open_idx = None;
    for (i, &b) in bytes.iter().enumerate().rev() {
        match b {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    open_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(open_idx) = open_idx else {
        return (name, None);
    };
    let inner = &name[open_idx + 1..name.len() - 1];
    let lhs = &name[..open_idx];

    if inner.is_empty() || lhs.is_empty() {
        return (name, None);
    }
    if inner.contains('.') || inner.contains('(') {
        return (name, None);
    }
    if !inner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return (name, None);
    }
    (lhs, Some(inner))
}

fn parse_evr(state: &ParserState, s: &str) -> EVR {
    let s = s.trim();
    let (epoch, rest) = match s.split_once(':') {
        Some((e, r)) if !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()) => {
            (e.parse::<u32>().ok(), r)
        }
        _ => (None, s),
    };
    let (version_str, release_str) = match rest.split_once('-') {
        Some((v, r)) => (v, Some(r)),
        None => (rest, None),
    };
    EVR {
        epoch,
        version: parse_body_as_text(state, version_str),
        release: release_str.map(|r| parse_body_as_text(state, r)),
    }
}

// ---------------------------------------------------------------------
// Rich / boolean parsing
// ---------------------------------------------------------------------

fn parse_rich_top(state: &ParserState, slice: &str) -> Result<DepExpr, ()> {
    let trimmed = slice.trim();
    let inner = match strip_outer_parens(trimmed) {
        Some(s) => s,
        None => {
            state.push_error_code(
                codes::E_RICH_DEP_UNBALANCED,
                format!("unbalanced parentheses in rich dependency `{trimmed}`"),
                None,
            );
            return Err(());
        }
    };
    parse_bool_expr(state, inner)
}

fn strip_outer_parens(s: &str) -> Option<&str> {
    if !s.starts_with('(') || !s.ends_with(')') {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    // Sanity check: the stripped slice must itself be balanced.
    let mut depth: i32 = 0;
    for b in inner.bytes() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth == 0 { Some(inner) } else { None }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpKind {
    And,
    Or,
    With,
    Without,
    If,
    Unless,
    Else,
}

const RICH_KEYWORDS: &[(&str, OpKind)] = &[
    ("and", OpKind::And),
    ("or", OpKind::Or),
    ("without", OpKind::Without),
    ("with", OpKind::With),
    ("if", OpKind::If),
    ("unless", OpKind::Unless),
    ("else", OpKind::Else),
];

/// Top-level operator scanner: finds keyword tokens at paren-depth 0,
/// surrounded by whitespace boundaries. Order in [`RICH_KEYWORDS`]
/// matters: longer prefixes (`without`) must precede shorter ones
/// (`with`).
fn scan_top_level_operators(inner: &str) -> Vec<(usize, usize, OpKind)> {
    let bytes = inner.as_bytes();
    let mut depth: i32 = 0;
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0 => {
                if let Some((end, op)) = match_keyword_at(inner, i) {
                    out.push((i, end, op));
                    i = end;
                } else {
                    i += 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    out
}

fn match_keyword_at(s: &str, i: usize) -> Option<(usize, OpKind)> {
    if i > 0 {
        let prev = s.as_bytes()[i - 1];
        if !matches!(prev, b' ' | b'\t') {
            return None;
        }
    }
    let rest = &s[i..];
    for (kw, op) in RICH_KEYWORDS {
        if rest.starts_with(kw) {
            let after = i + kw.len();
            match s.as_bytes().get(after).copied() {
                Some(b' ') | Some(b'\t') => return Some((after, *op)),
                None => return Some((after, *op)),
                _ => continue,
            }
        }
    }
    None
}

fn parse_bool_expr(state: &ParserState, inner: &str) -> Result<DepExpr, ()> {
    let ops = scan_top_level_operators(inner);

    if ops.is_empty() {
        return parse_operand(state, inner);
    }

    // If/unless take precedence: they reshape the operand list entirely.
    if ops
        .iter()
        .any(|(_, _, op)| matches!(op, OpKind::If | OpKind::Unless))
    {
        return parse_if_unless(state, inner, &ops);
    }

    if ops.iter().any(|(_, _, op)| matches!(op, OpKind::Else)) {
        state.push_error_code(
            codes::E_ELSE_WITHOUT_IF,
            format!("`else` without `if` or `unless` in rich dep `{inner}`"),
            None,
        );
        return Err(());
    }

    let first_op = ops[0].2;
    if ops.iter().any(|(_, _, op)| *op != first_op) {
        state.push_error_code(
            codes::E_RICH_DEP_MIXED_OPS,
            format!(
                "mixed operators in rich dep `{inner}`: only one operator allowed per parenthesized level"
            ),
            None,
        );
        return Err(());
    }

    let operands = collect_operands(state, inner, &ops)?;

    if matches!(first_op, OpKind::Without) {
        if operands.len() != 2 {
            state.push_error_code(
                codes::E_RICH_DEP_WITHOUT_ARITY,
                format!("`without` requires exactly two operands in `{inner}`"),
                None,
            );
            return Err(());
        }
        let mut it = operands.into_iter();
        let left = it.next().unwrap();
        let right = it.next().unwrap();
        return Ok(DepExpr::Rich(Box::new(BoolDep::Without {
            left: Box::new(left),
            right: Box::new(right),
        })));
    }

    let bool_dep = match first_op {
        OpKind::And => BoolDep::And(operands),
        OpKind::Or => BoolDep::Or(operands),
        OpKind::With => BoolDep::With(operands),
        _ => unreachable!("guarded above"),
    };
    Ok(DepExpr::Rich(Box::new(bool_dep)))
}

fn collect_operands(
    state: &ParserState,
    inner: &str,
    ops: &[(usize, usize, OpKind)],
) -> Result<Vec<DepExpr>, ()> {
    let mut operands = Vec::with_capacity(ops.len() + 1);
    let mut last = 0usize;
    for (start, end, _) in ops {
        let chunk = inner[last..*start].trim();
        operands.push(parse_operand(state, chunk)?);
        last = *end;
    }
    operands.push(parse_operand(state, inner[last..].trim())?);
    Ok(operands)
}

fn parse_if_unless(
    state: &ParserState,
    inner: &str,
    ops: &[(usize, usize, OpKind)],
) -> Result<DepExpr, ()> {
    let primary_idx = ops
        .iter()
        .position(|(_, _, op)| matches!(op, OpKind::If | OpKind::Unless))
        .expect("guarded by caller");
    let primary = ops[primary_idx];

    // Optional else after the primary if/unless.
    let else_rel = ops[primary_idx + 1..]
        .iter()
        .position(|(_, _, op)| matches!(op, OpKind::Else));

    // Reject extra unrelated operators in the expression.
    for (i, (_, _, op)) in ops.iter().enumerate() {
        if i == primary_idx {
            continue;
        }
        if let Some(rel) = else_rel {
            if i == primary_idx + 1 + rel {
                continue;
            }
        }
        state.push_error_code(
            codes::E_UNEXPECTED_OP_IF_UNLESS,
            format!(
                "unexpected `{}` in if/unless expression `{inner}`",
                op_label(*op)
            ),
            None,
        );
        return Err(());
    }

    let then_chunk = inner[..primary.0].trim();
    let then_expr = parse_operand(state, then_chunk)?;

    let (cond_chunk, else_expr) = if let Some(rel) = else_rel {
        let else_op = ops[primary_idx + 1 + rel];
        let cond_chunk = inner[primary.1..else_op.0].trim();
        let else_chunk = inner[else_op.1..].trim();
        (cond_chunk, Some(parse_operand(state, else_chunk)?))
    } else {
        (inner[primary.1..].trim(), None)
    };

    let cond_expr = parse_operand(state, cond_chunk)?;

    let bd = match primary.2 {
        OpKind::If => BoolDep::If {
            cond: Box::new(cond_expr),
            then: Box::new(then_expr),
            otherwise: else_expr.map(Box::new),
        },
        OpKind::Unless => BoolDep::Unless {
            cond: Box::new(cond_expr),
            then: Box::new(then_expr),
            otherwise: else_expr.map(Box::new),
        },
        _ => unreachable!(),
    };
    Ok(DepExpr::Rich(Box::new(bd)))
}

fn op_label(op: OpKind) -> &'static str {
    match op {
        OpKind::And => "and",
        OpKind::Or => "or",
        OpKind::With => "with",
        OpKind::Without => "without",
        OpKind::If => "if",
        OpKind::Unless => "unless",
        OpKind::Else => "else",
    }
}

fn parse_operand(state: &ParserState, s: &str) -> Result<DepExpr, ()> {
    let s = s.trim();
    if s.is_empty() {
        state.push_error_code(
            codes::E_RICH_DEP_EMPTY_OPERAND,
            "empty operand in rich dependency",
            None,
        );
        return Err(());
    }
    if s.starts_with('(') {
        return parse_rich_top(state, s);
    }
    Ok(DepExpr::Atom(parse_atom(state, s)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(slice: &str) -> (DepExpr, ParserState) {
        let state = ParserState::new();
        let r = parse_dep_expr(&state, slice).expect("parses");
        (r, state)
    }

    fn parse_atom_only(slice: &str) -> DepAtom {
        match parse(slice).0 {
            DepExpr::Atom(a) => a,
            other => panic!("expected atom, got {other:?}"),
        }
    }

    #[test]
    fn atom_bare_name() {
        let a = parse_atom_only("glibc");
        assert_eq!(a.name.literal_str(), Some("glibc"));
        assert!(a.arch.is_none());
        assert!(a.constraint.is_none());
    }

    #[test]
    fn atom_with_version() {
        let a = parse_atom_only("glibc >= 2.34");
        assert_eq!(a.name.literal_str(), Some("glibc"));
        let c = a.constraint.expect("constraint");
        assert_eq!(c.op, VerOp::Ge);
        assert_eq!(c.evr.version.literal_str(), Some("2.34"));
        assert!(c.evr.epoch.is_none());
        assert!(c.evr.release.is_none());
    }

    #[test]
    fn atom_with_epoch_and_release() {
        let a = parse_atom_only("perl-DBI = 9:1.643-1.fc40");
        let c = a.constraint.unwrap();
        assert_eq!(c.evr.epoch, Some(9));
        assert_eq!(c.evr.version.literal_str(), Some("1.643"));
        assert_eq!(c.evr.release.unwrap().literal_str(), Some("1.fc40"));
    }

    #[test]
    fn atom_arch_qualifier() {
        let a = parse_atom_only("kernel(x86-64) >= 5.0");
        assert_eq!(a.name.literal_str(), Some("kernel"));
        assert_eq!(a.arch.unwrap().literal_str(), Some("x86-64"));
        assert_eq!(a.constraint.unwrap().evr.version.literal_str(), Some("5.0"));
    }

    #[test]
    fn atom_provider_style_no_arch() {
        let a = parse_atom_only("pkgconfig(glib-2.0)");
        assert_eq!(a.name.literal_str(), Some("pkgconfig(glib-2.0)"));
        assert!(a.arch.is_none());
    }

    #[test]
    fn atom_perl_provider_with_colons() {
        let a = parse_atom_only("perl(File::Basename)");
        assert_eq!(a.name.literal_str(), Some("perl(File::Basename)"));
        assert!(a.arch.is_none());
    }

    #[test]
    fn atom_file_dep() {
        let a = parse_atom_only("/usr/bin/awk");
        assert_eq!(a.name.literal_str(), Some("/usr/bin/awk"));
    }

    #[test]
    fn atom_tight_operator() {
        // No whitespace around `>=`.
        let a = parse_atom_only("foo>=1.0");
        assert_eq!(a.name.literal_str(), Some("foo"));
        let c = a.constraint.unwrap();
        assert_eq!(c.op, VerOp::Ge);
        assert_eq!(c.evr.version.literal_str(), Some("1.0"));
    }

    #[test]
    fn rich_and() {
        let (e, _) = parse("(foo and bar and baz)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::And(v) => assert_eq!(v.len(), 3),
                other => panic!("expected And, got {other:?}"),
            },
            other => panic!("expected rich, got {other:?}"),
        }
    }

    #[test]
    fn rich_or() {
        let (e, _) = parse("(a or b)");
        assert!(matches!(e, DepExpr::Rich(b) if matches!(*b, BoolDep::Or(_))));
    }

    #[test]
    fn rich_with() {
        let (e, _) = parse("(a with b)");
        assert!(matches!(e, DepExpr::Rich(b) if matches!(*b, BoolDep::With(_))));
    }

    #[test]
    fn rich_without() {
        let (e, _) = parse("(a without b)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::Without { .. } => {}
                other => panic!("expected Without, got {other:?}"),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn rich_if_else() {
        let (e, _) = parse("(then if cond else otherwise)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::If { otherwise, .. } => assert!(otherwise.is_some()),
                other => panic!("{other:?}"),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn rich_if_no_else() {
        let (e, _) = parse("(then if cond)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::If { otherwise, .. } => assert!(otherwise.is_none()),
                other => panic!("{other:?}"),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn rich_unless_else() {
        let (e, _) = parse("(then unless cond else fallback)");
        match e {
            DepExpr::Rich(b) => assert!(matches!(
                *b,
                BoolDep::Unless {
                    otherwise: Some(_),
                    ..
                }
            )),
            _ => panic!(),
        }
    }

    #[test]
    fn rich_nested() {
        let (e, _) = parse("((a and b) or c)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::Or(v) => {
                    assert_eq!(v.len(), 2);
                    assert!(matches!(&v[0], DepExpr::Rich(_)));
                    assert!(matches!(&v[1], DepExpr::Atom(_)));
                }
                other => panic!("{other:?}"),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn rich_mixed_operators_errors() {
        let state = ParserState::new();
        let r = parse_dep_expr(&state, "(a and b or c)");
        assert!(r.is_err());
        assert!(state.has_errors());
    }

    #[test]
    fn rich_atom_with_version_inside() {
        let (e, _) = parse("(foo >= 1.0 and bar)");
        match e {
            DepExpr::Rich(b) => match *b {
                BoolDep::And(v) => {
                    assert_eq!(v.len(), 2);
                    if let DepExpr::Atom(a) = &v[0] {
                        assert_eq!(a.name.literal_str(), Some("foo"));
                        assert!(a.constraint.is_some());
                    } else {
                        panic!("expected atom");
                    }
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }
}
