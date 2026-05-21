//! Parsers for section headers and the structural bodies that Stage 2
//! produces (`%description`, `%package`).
//!
//! Other section names (`%prep`, `%build`, `%files`, `%changelog`, …)
//! are detected here only for the purpose of stopping the parent body
//! parser — their structural bodies remain Stage 3 work and the
//! top-level loop in `entry.rs` falls back to the deferred-placeholder
//! path for them.

use nom::{IResult, error::ErrorKind, error_position};

use crate::ast::{
    BuildScriptKind, PackageName, PreambleContent, Section, ShellBody, ShellCondBranch,
    ShellCondElse, ShellConditional, Span, SubpkgRef, Text, TextBody, TextSegment,
};
use crate::parse_result::codes;

use super::cond::{
    SHELL_ELIF_KEYWORDS, SHELL_IF_KEYWORDS, parse_branch_head, starts_with_keyword,
};
use super::input::{Input, span_at, span_between, span_for_line};
use super::preamble::parse_preamble_content;
use super::state::ParserState;
use super::text::{parse_body_as_text, parse_text};
use super::util::{line_terminator, physical_line, space0, space1};

/// DoS guard: refuse to track more than this many nested `%if` blocks in a
/// shell body. Pathologically deep nesting is best-effort recovery only.
const MAX_SHELL_COND_DEPTH: usize = 64;

/// Section header names that introduce a top-level section. Order does
/// not matter except that longer-prefix names must be tried first when
/// they overlap (`%description` vs `%desc`) — none currently do.
pub(crate) const SECTION_HEADERS: &[&str] = &[
    "%description",
    "%package",
    "%prep",
    "%conf",
    "%build",
    "%install",
    "%check",
    "%clean",
    "%generate_buildrequires",
    "%files",
    "%changelog",
    "%sourcelist",
    "%patchlist",
    "%verify",
    "%sepolicy",
    "%pre",
    "%post",
    "%preun",
    "%postun",
    "%pretrans",
    "%posttrans",
    "%preuntrans",
    "%postuntrans",
    "%triggerprein",
    "%triggerin",
    "%triggerun",
    "%triggerpostun",
    "%filetriggerin",
    "%filetriggerun",
    "%filetriggerpostun",
    "%transfiletriggerin",
    "%transfiletriggerun",
    "%transfiletriggerpostun",
];

/// Returns the canonical name (e.g. `"%description"`) when the cursor
/// (after any leading whitespace) sits on a recognized section header,
/// else `None`. Section names must be followed by whitespace, EOL, EOF,
/// or `-` (option).
pub fn peek_section_header(input: Input<'_>) -> Option<&'static str> {
    let after_ws = match space0(input) {
        Ok((r, _)) => r,
        Err(_) => return None,
    };
    let frag = *after_ws.fragment();
    for header in SECTION_HEADERS {
        if let Some(rest) = frag.strip_prefix(header) {
            match rest.chars().next() {
                None | Some(' ' | '\t' | '\n' | '\r' | '-') => return Some(*header),
                _ => {}
            }
        }
    }
    None
}

/// Parse a structural section if the cursor sits on a section header that
/// Stage 2 knows how to handle. Returns `Ok((rest, None))` when the
/// header is recognized but its body is *not* yet implemented (the
/// caller should fall back to the deferred-placeholder path).
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "trace", skip(state, input))
)]
pub fn parse_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Option<Section<Span>>> {
    let header = match peek_section_header(input) {
        Some(h) => h,
        None => {
            return Err(nom::Err::Error(error_position!(input, ErrorKind::Tag)));
        }
    };
    match header {
        "%description" => {
            let (rest, sec) = parse_description_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%package" => {
            let (rest, sec) = parse_package_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%prep" => parse_build_script(state, input, "%prep", BuildScriptKind::Prep).map(some),
        "%conf" => parse_build_script(state, input, "%conf", BuildScriptKind::Conf).map(some),
        "%build" => parse_build_script(state, input, "%build", BuildScriptKind::Build).map(some),
        "%install" => {
            parse_build_script(state, input, "%install", BuildScriptKind::Install).map(some)
        }
        "%check" => parse_build_script(state, input, "%check", BuildScriptKind::Check).map(some),
        "%clean" => parse_build_script(state, input, "%clean", BuildScriptKind::Clean).map(some),
        "%generate_buildrequires" => parse_build_script(
            state,
            input,
            "%generate_buildrequires",
            BuildScriptKind::GenerateBuildRequires,
        )
        .map(some),
        "%verify" => parse_verify_section(state, input).map(some),
        "%sepolicy" => parse_sepolicy_section(state, input).map(some),
        "%sourcelist" => {
            parse_list_section(state, input, "%sourcelist", ListKind::Source).map(some)
        }
        "%patchlist" => parse_list_section(state, input, "%patchlist", ListKind::Patch).map(some),
        "%files" => {
            let (rest, sec) = super::files::parse_files_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%changelog" => {
            let (rest, sec) = super::changelog::parse_changelog_section(state, input)?;
            Ok((rest, Some(sec)))
        }
        "%pre" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%pre",
            crate::ast::ScriptletKind::Pre,
        )
        .map(some),
        "%post" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%post",
            crate::ast::ScriptletKind::Post,
        )
        .map(some),
        "%preun" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%preun",
            crate::ast::ScriptletKind::Preun,
        )
        .map(some),
        "%postun" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%postun",
            crate::ast::ScriptletKind::Postun,
        )
        .map(some),
        "%pretrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%pretrans",
            crate::ast::ScriptletKind::Pretrans,
        )
        .map(some),
        "%posttrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%posttrans",
            crate::ast::ScriptletKind::Posttrans,
        )
        .map(some),
        "%preuntrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%preuntrans",
            crate::ast::ScriptletKind::Preuntrans,
        )
        .map(some),
        "%postuntrans" => super::scriptlet::parse_scriptlet_section(
            state,
            input,
            "%postuntrans",
            crate::ast::ScriptletKind::Postuntrans,
        )
        .map(some),
        "%triggerprein" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerprein",
            crate::ast::TriggerKind::Prein,
        )
        .map(some),
        "%triggerin" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerin",
            crate::ast::TriggerKind::In,
        )
        .map(some),
        "%triggerun" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerun",
            crate::ast::TriggerKind::Un,
        )
        .map(some),
        "%triggerpostun" => super::scriptlet::parse_trigger_section(
            state,
            input,
            "%triggerpostun",
            crate::ast::TriggerKind::Postun,
        )
        .map(some),
        "%filetriggerin" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerin",
            crate::ast::FileTriggerKind::In,
        )
        .map(some),
        "%filetriggerun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerun",
            crate::ast::FileTriggerKind::Un,
        )
        .map(some),
        "%filetriggerpostun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%filetriggerpostun",
            crate::ast::FileTriggerKind::Postun,
        )
        .map(some),
        "%transfiletriggerin" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerin",
            crate::ast::FileTriggerKind::TransIn,
        )
        .map(some),
        "%transfiletriggerun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerun",
            crate::ast::FileTriggerKind::TransUn,
        )
        .map(some),
        "%transfiletriggerpostun" => super::scriptlet::parse_file_trigger_section(
            state,
            input,
            "%transfiletriggerpostun",
            crate::ast::FileTriggerKind::TransPostun,
        )
        .map(some),
        _ => Ok((input, None)),
    }
}

fn some<I, T>((rest, value): (I, T)) -> (I, Option<T>) {
    (rest, Some(value))
}

// ---------------------------------------------------------------------
// Shell-body helper shared by build-scripts/scriptlets/triggers/etc.
// ---------------------------------------------------------------------

/// Consume body lines (one `Text` per physical line, with macros parsed)
/// until the next recognized section header or EOF. Used by build-script
/// and scriptlet/trigger sections.
///
/// In addition to the flat line list, this pass detects `%if`/`%elif`/
/// `%else`/`%endif` directives appearing at the head of any physical line
/// and surfaces them as a *flat* list of [`ShellConditional`] entries (one
/// per `%if`…`%endif` block, regardless of nesting depth — analyses derive
/// parent/child relations from `Span` containment when they need to). The
/// `%if`/`%endif` directive lines themselves remain in [`ShellBody::lines`]
/// as plain `Text`, preserving backward compatibility for the dozens of
/// downstream consumers that walk `lines` flatly.
pub(crate) fn collect_shell_body_until_section_header<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, ShellBody<Span>) {
    let mut cursor = input;
    let mut lines: Vec<Text> = Vec::new();
    let mut conditionals: Vec<ShellConditional<Span>> = Vec::new();
    // Stack of in-progress `%if`...`%endif` blocks. `branches` accumulates
    // the `%if`/`%elif*` heads; their final spans (extending to the next
    // sibling or `%endif`) are patched in once we see that sibling.
    let mut stack: Vec<PendingShellCond> = Vec::new();
    // Rate-limit the depth-overflow warning: emit once per shell body, no
    // matter how many `%if`s exceed `MAX_SHELL_COND_DEPTH`.
    let mut depth_warned = false;
    // Rewind anchor for the "%if wraps the next section" idiom: when the
    // shell body grows the first unclosed `%if`, snapshot the cursor + the
    // number of body lines collected so far. If a section header shows up
    // before the matching `%endif`, the `%if` was section-level all along —
    // we rewind the body to just before it and let the outer spec-level
    // parser handle the conditional. See the inline comment on the rewind
    // branch below for the full rationale and the tests
    // `shell_body_rewinds_when_section_header_appears_inside_if` /
    // `shell_body_preserves_properly_closed_if_then_section`.
    let mut rewind_anchor: Option<(Input<'a>, usize)> = None;

    while !cursor.fragment().is_empty() {
        // Orphan `%endif` / `%else` / `%elif` with NO pending shell-level
        // `%if` on the stack means the directive closes an outer
        // top-level conditional (e.g. the body of a `%postun` that lives
        // inside `%if %plperl ... %endif`). Hand it back to the spec-level
        // parser by breaking — the rewind in the section-header arm
        // below already covers the symmetric "%if opens an outer block"
        // case.
        if stack.is_empty()
            && let Some(frag) = first_nonspace_keyword(cursor)
            && (starts_with_keyword(frag, "%endif")
                || starts_with_keyword(frag, "%else")
                || starts_with_any_elif_head(frag))
        {
            break;
        }
        if peek_section_header(cursor).is_some() {
            // Section-level `%if` wrapping subsequent sections (common rpm
            // idiom in real specs):
            //
            //     %postun server
            //     /sbin/ldconfig
            //
            //     %if %plperl
            //     %post -p /sbin/ldconfig plperl
            //     %postun -p /sbin/ldconfig plperl
            //     %endif
            //
            // The previous implementation greedily kept consuming the
            // scriptlet body, saw `%if %plperl` as a shell-level
            // conditional, then choked on the next `%post` header (which
            // breaks the loop with the conditional still unclosed) and
            // emitted a spurious "unterminated `%if`" error. The fix:
            // when we detect a section header AND an outer `%if` is still
            // open, retract the body back to just before that `%if` and
            // let the spec-level parser pick up the conditional —
            // sections inside the branches are perfectly legal at the
            // spec level.
            if let Some((rewind_cursor, rewind_lines_len)) = rewind_anchor {
                let rewind_offset = rewind_cursor.location_offset();
                lines.truncate(rewind_lines_len);
                // Drop any nested conditionals that were finalised
                // between the outer `%if` head and the rewind point —
                // they belong to the section-level block we're handing
                // back to the spec parser.
                conditionals.retain(|c| c.data.start_byte < rewind_offset);
                stack.clear();
                cursor = rewind_cursor;
            }
            break;
        }
        let here = cursor;
        let (after, line_input) = match physical_line(here) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after.location_offset() == here.location_offset() {
            break;
        }

        let stack_was_empty = stack.is_empty();
        scan_shell_cond_directive(
            state,
            here,
            line_input,
            after,
            &mut stack,
            &mut conditionals,
            &mut depth_warned,
        );
        // Track the rewind anchor: empty→non-empty transitions record
        // (cursor before the line, current `lines.len()`); non-empty→
        // empty clears the anchor since every `%if` we know about is
        // now properly closed.
        match (stack_was_empty, stack.is_empty()) {
            (true, false) => rewind_anchor = Some((here, lines.len())),
            (false, true) => rewind_anchor = None,
            _ => {}
        }

        let line = parse_body_as_text(state, line_input.fragment());
        lines.push(line);
        cursor = after;
    }

    // Trim trailing empty lines from the source-text view. (Conditionals are
    // never trailing-empty — they always end on `%endif`, so they're unaffected.)
    while matches!(lines.last(), Some(t) if is_empty_text(t)) {
        lines.pop();
    }

    // Any unterminated `%if`s left on the stack get a diagnostic + a recovery
    // emission so analysers still see partial structure rather than nothing.
    while let Some(pending) = stack.pop() {
        state.push_error_code(
            codes::E_UNTERMINATED_CONDITIONAL,
            "unterminated `%if` inside shell body (no matching `%endif`)",
            Some(pending.head_span),
        );
        if let Some(cond) = finalize_pending(pending, cursor.location_offset(), cursor.location_line()) {
            conditionals.push(cond);
        }
    }
    // `stack`-pop yields children before their parents (LIFO), so sort by
    // `start_byte` to restore source order (outer block before its nested ones).
    // start_byte is unique per block, stability not needed.
    conditionals.sort_unstable_by_key(|c| c.data.start_byte);

    (cursor, ShellBody { lines, conditionals })
}

/// In-progress `%if`/`%elif*`/`%else` block awaiting its closing `%endif`.
/// Branch spans are patched as later siblings or the `%endif` are seen.
struct PendingShellCond {
    /// Span of the `%if` head line, used as the start anchor for the
    /// outer-block span when the closing `%endif` (or recovery) finalises it.
    head_span: Span,
    /// Branches collected so far, last entry possibly without its body-end byte yet.
    branches: Vec<ShellCondBranch<Span>>,
    /// `%else` clause once observed.
    otherwise: Option<ShellCondElse<Span>>,
    /// `true` once `%else` was consumed; subsequent branches go into `otherwise`.
    in_else: bool,
}

/// Look at one physical line: if it starts with a conditional directive,
/// update `stack`/`conditionals` accordingly. Non-directive lines are no-ops
/// here — the caller still pushes the text into `lines`.
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "trace", skip(state, stack, conditionals, depth_warned))
)]
fn scan_shell_cond_directive<'a>(
    state: &ParserState,
    line_start: Input<'a>,
    line_input: Input<'a>,
    after_line: Input<'a>,
    stack: &mut Vec<PendingShellCond>,
    conditionals: &mut Vec<ShellConditional<Span>>,
    depth_warned: &mut bool,
) {
    let frag = line_input.fragment().trim_start();
    // Cheap-reject: every conditional directive starts with `%`.
    if !frag.starts_with('%') {
        return;
    }
    let head_line = line_start.location_line();
    let line_span = span_for_line(&line_start, &line_input);

    if starts_with_any_if_head(frag) {
        // Bounded recursion: refuse to track more than MAX_SHELL_COND_DEPTH nested %ifs.
        if stack.len() >= MAX_SHELL_COND_DEPTH {
            // Rate-limit via `depth_warned`: a pathologically deep input could
            // otherwise emit hundreds of identical warnings.
            if !*depth_warned {
                state.push_warning_code(
                    codes::W_UNTERMINATED_MACRO,
                    "shell-body `%if` nesting exceeds MAX_SHELL_COND_DEPTH; deeper structure ignored",
                    Some(line_span),
                );
                *depth_warned = true;
            }
            return;
        }
        // `%if` / `%ifarch` / `%ifnarch` / `%ifos` / `%ifnos`. Try to parse
        // the head expression starting at the beginning of the original
        // line so spans are anchored to source positions. On parse failure
        // (malformed grammar), surface a diagnostic — silent skip would hide
        // partially formed structure from analyses.
        if let Ok((_, (kind, expr))) = parse_branch_head(line_start, false) {
            stack.push(PendingShellCond {
                head_span: line_span,
                branches: vec![ShellCondBranch {
                    kind,
                    expr,
                    data: line_span,
                    head_line,
                }],
                otherwise: None,
                in_else: false,
            });
        } else {
            state.push_warning_code(
                codes::W_UNTERMINATED_MACRO,
                "could not parse `%if` head expression; structure may be incomplete",
                Some(line_span),
            );
        }
        return;
    }

    if starts_with_any_elif_head(frag) {
        let Some(top) = stack.last_mut() else {
            return;
        };
        if top.in_else {
            // `%elif` after `%else` — RPM semantics: rolled into the else body.
            state.push_warning_code(
                codes::W_ELIF_AFTER_ELSE,
                "`%elif` appeared after `%else`; treated as part of the `%else` body",
                Some(line_span),
            );
            return;
        }
        if let Ok((_, (kind, expr))) = parse_branch_head(line_start, true) {
            patch_last_branch_end(
                top,
                line_input.location_offset(),
                line_start.location_line(),
                u32::try_from(line_start.get_column()).unwrap_or(u32::MAX),
            );
            top.branches.push(ShellCondBranch {
                kind,
                expr,
                data: line_span,
                head_line,
            });
        } else {
            state.push_warning_code(
                codes::W_UNTERMINATED_MACRO,
                "could not parse `%elif` head expression; structure may be incomplete",
                Some(line_span),
            );
        }
        return;
    }

    if starts_with_keyword(frag, "%else") {
        let Some(top) = stack.last_mut() else {
            return;
        };
        if top.in_else {
            state.push_warning_code(
                codes::W_MULTIPLE_ELSE,
                "conditional block contains more than one `%else`",
                Some(line_span),
            );
        }
        patch_last_branch_end(
            top,
            line_input.location_offset(),
            line_start.location_line(),
            u32::try_from(line_start.get_column()).unwrap_or(u32::MAX),
        );
        top.otherwise = Some(ShellCondElse {
            data: line_span,
            head_line,
        });
        top.in_else = true;
        return;
    }

    if starts_with_keyword(frag, "%endif") {
        let Some(mut pending) = stack.pop() else {
            // Stray `%endif`: closest existing code is the inverse
            // "unterminated conditional"; reuse it rather than inventing one.
            state.push_error_code(
                codes::E_UNTERMINATED_CONDITIONAL,
                "`%endif` without matching `%if` inside shell body",
                Some(line_span),
            );
            return;
        };
        // Extend the last branch (or `%else`) to cover up to `%endif`'s end.
        let endif_end_byte = line_input.location_offset() + line_input.fragment().len();
        let line_len_for_patch = u32::try_from(line_input.fragment().len()).unwrap_or(u32::MAX);
        let line_start_col_for_patch =
            u32::try_from(line_start.get_column()).unwrap_or(u32::MAX);
        patch_last_branch_end(
            &mut pending,
            endif_end_byte,
            line_start.location_line(),
            line_len_for_patch.saturating_add(line_start_col_for_patch),
        );
        let line_len_u32 = u32::try_from(line_input.fragment().len()).unwrap_or(u32::MAX);
        let line_start_col_u32 = u32::try_from(line_start.get_column()).unwrap_or(u32::MAX);
        let after_line_col_u32 = u32::try_from(after_line.get_column()).unwrap_or(u32::MAX);
        if let Some(els) = pending.otherwise.as_mut() {
            els.data = span_extend(
                els.data,
                endif_end_byte,
                line_start.location_line(),
                line_len_u32.saturating_add(line_start_col_u32),
            );
        }
        let outer = span_extend(
            pending.head_span,
            endif_end_byte,
            after_line.location_line(),
            after_line_col_u32,
        );
        conditionals.push(ShellConditional {
            branches: pending.branches,
            otherwise: pending.otherwise,
            data: outer,
        });
    }
}

fn starts_with_any_if_head(frag: &str) -> bool {
    SHELL_IF_KEYWORDS
        .iter()
        .any(|kw| starts_with_keyword(frag, kw))
}

/// Strip leading whitespace and return the rest as `Some(&str)` when it
/// begins with a `%` keyword — caller pairs this with `starts_with_*`
/// helpers to peek the current line's directive without consuming.
/// Returns `None` for blank / non-keyword lines so the caller short-
/// circuits quickly.
fn first_nonspace_keyword<'a>(cursor: Input<'a>) -> Option<&'a str> {
    let frag = (*cursor.fragment()).trim_start_matches([' ', '\t']);
    if frag.starts_with('%') { Some(frag) } else { None }
}

fn starts_with_any_elif_head(frag: &str) -> bool {
    SHELL_ELIF_KEYWORDS
        .iter()
        .any(|kw| starts_with_keyword(frag, kw))
}

/// Patch the most recently added branch's span to cover its body up to
/// `end_byte` / (`end_line`, `end_col`). Called when a sibling
/// (`%elif`/`%else`) or the closing `%endif` is seen — the new directive's
/// own line/column become the branch's end-of-span so downstream consumers
/// (e.g. `matrix impact`'s per-profile filter) can derive correct body
/// line ranges. The previous version of this helper kept the stale
/// `end_line` from the original `%if` head, producing a degenerate
/// single-line range that caused active-line filtering to silently no-op.
fn patch_last_branch_end(
    pending: &mut PendingShellCond,
    end_byte: usize,
    end_line: u32,
    end_col: u32,
) {
    if pending.in_else {
        // Body of `%else` already tracked separately via `otherwise.data`.
        return;
    }
    let Some(last) = pending.branches.last_mut() else {
        return;
    };
    if end_byte >= last.data.start_byte {
        last.data = span_extend(last.data, end_byte, end_line, end_col);
    }
}

/// Build a `Span` that keeps the start position of `start` but extends to the
/// supplied end byte / line / column. Centralises the otherwise-repetitive
/// 6-arg `Span::new` calls used throughout the shell-body conditional scanner.
fn span_extend(start: Span, end_byte: usize, end_line: u32, end_col: u32) -> Span {
    Span::new(
        start.start_byte,
        end_byte,
        start.start_line,
        start.start_column,
        end_line,
        end_col,
    )
}

/// Best-effort recovery for an unterminated `%if`: build a [`ShellConditional`]
/// covering everything seen so far so analyses surface partial structure.
fn finalize_pending(
    pending: PendingShellCond,
    end_byte: usize,
    end_line: u32,
) -> Option<ShellConditional<Span>> {
    if pending.branches.is_empty() && pending.otherwise.is_none() {
        return None;
    }
    let outer = span_extend(
        pending.head_span,
        end_byte,
        end_line,
        // recovery: end_column reset to start-of-line (true position unknown
        // because the matching `%endif` is missing).
        1,
    );
    Some(ShellConditional {
        branches: pending.branches,
        otherwise: pending.otherwise,
        data: outer,
    })
}

// ---------------------------------------------------------------------
// Build-script handlers (%prep / %conf / %build / %install / %check /
// %clean / %generate_buildrequires)
// ---------------------------------------------------------------------

fn parse_build_script<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: BuildScriptKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    // Build-scripts have no header args in practice; consume the rest of
    // the header line (which may include `# trailing comment`).
    let (after_header, _) = line_terminator(after_kw)?;

    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::BuildScript {
            kind,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %verify
// ---------------------------------------------------------------------

fn parse_verify_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%verify".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::Verify {
            subpkg,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %sepolicy
// ---------------------------------------------------------------------

fn parse_sepolicy_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%sepolicy".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;
    let (after_body, body) = collect_shell_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);
    Ok((
        after_body,
        Section::Sepolicy {
            subpkg,
            body,
            data: span,
        },
    ))
}

// ---------------------------------------------------------------------
// %sourcelist / %patchlist
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Source,
    Patch,
}

fn parse_list_section<'a>(
    state: &ParserState,
    input: Input<'a>,
    keyword: &str,
    kind: ListKind,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, keyword.len());
    let (after_header, _) = line_terminator(after_kw)?;

    let mut cursor = after_header;
    let mut entries: Vec<Text> = Vec::new();
    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let here = cursor;
        let (after, line_input) = match physical_line(here) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after.location_offset() == here.location_offset() {
            break;
        }
        let frag = line_input.fragment().trim();
        if !frag.is_empty() && !frag.starts_with('#') {
            entries.push(parse_body_as_text(state, frag));
        }
        cursor = after;
    }

    let span = span_between(&start, &cursor);
    Ok((
        cursor,
        match kind {
            ListKind::Source => Section::SourceList {
                entries,
                data: span,
            },
            ListKind::Patch => Section::PatchList {
                entries,
                data: span,
            },
        },
    ))
}

// ---------------------------------------------------------------------
// %description
// ---------------------------------------------------------------------

fn parse_description_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%description".len());
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let (after_header, _) = line_terminator(after_args)?;

    let (after_body, body) = collect_text_body_until_section_header(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Description {
            subpkg,
            body,
            data: span,
        },
    ))
}

fn collect_text_body_until_section_header<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, TextBody) {
    let mut cursor = input;
    let mut lines: Vec<Text> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        let line_start = cursor;
        let (after_line_content, line_input) = match physical_line(cursor) {
            Ok(r) => r,
            Err(_) => break,
        };
        if after_line_content.location_offset() == line_start.location_offset() {
            break;
        }
        // Parse the textual portion of this physical line as a Text.
        let line_text = parse_body_as_text(state, line_input.fragment());
        lines.push(line_text);
        cursor = after_line_content;
    }

    // Trim trailing empty lines (cosmetic: a body that immediately
    // precedes the next section header would otherwise carry a stray
    // empty line just to satisfy the source separator).
    while matches!(lines.last(), Some(t) if is_empty_text(t)) {
        lines.pop();
    }

    (cursor, TextBody { lines })
}

fn is_empty_text(t: &Text) -> bool {
    t.segments
        .iter()
        .all(|s| matches!(s, TextSegment::Literal(s) if s.trim().is_empty()))
}

// ---------------------------------------------------------------------
// %package
// ---------------------------------------------------------------------

fn parse_package_section<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> IResult<Input<'a>, Section<Span>> {
    let start = input;
    let (after_ws, _) = space0(input)?;
    let (after_kw, _) = nom::Input::take_split(&after_ws, "%package".len());

    // %package requires a name argument.
    let (after_args, subpkg) = parse_header_args(state, after_kw);
    let name_arg = match subpkg {
        Some(SubpkgRef::Absolute(t)) => PackageName::Absolute(t),
        Some(SubpkgRef::Relative(t)) => PackageName::Relative(t),
        None => {
            state.push_error_code(
                codes::E_PACKAGE_NEEDS_NAME,
                "%package requires a subpackage name argument",
                Some(span_at(&after_args)),
            );
            // Recover with an empty name so the rest of the file still
            // parses; consumers see the diagnostic.
            PackageName::Relative(Text::new())
        }
    };
    let (after_header, _) = line_terminator(after_args)?;

    let (after_body, content) = collect_package_body(state, after_header);
    let span = span_between(&start, &after_body);

    Ok((
        after_body,
        Section::Package {
            name_arg,
            content,
            data: span,
        },
    ))
}

fn collect_package_body<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Vec<PreambleContent<Span>>) {
    let mut cursor = input;
    let mut content: Vec<PreambleContent<Span>> = Vec::new();

    while !cursor.fragment().is_empty() {
        if peek_section_header(cursor).is_some() {
            break;
        }
        match parse_preamble_content(state, cursor) {
            Ok((rest, items)) => {
                if rest.location_offset() == cursor.location_offset() {
                    // No progress — bail to avoid infinite loop.
                    break;
                }
                content.extend(items);
                cursor = rest;
            }
            Err(_) => {
                // Unrecognized line inside %package body — consume one
                // physical line with a warning so the body parser stays
                // productive.
                let here = cursor;
                let (after, line_text) = match physical_line(here) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                if after.location_offset() == here.location_offset() {
                    break;
                }
                state.push_warning_code(
                    codes::W_LINE_NOT_RECOGNIZED_IN_PACKAGE,
                    "line not recognized inside %package body",
                    Some(span_for_line(&here, &line_text)),
                );
                cursor = after;
            }
        }
    }

    (cursor, content)
}

// ---------------------------------------------------------------------
// `-n NAME` and bare-NAME header argument parsing
// ---------------------------------------------------------------------

fn parse_header_args<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> (Input<'a>, Option<SubpkgRef>) {
    // Consume optional inline whitespace.
    let (cursor, _) = match space0(input) {
        Ok(r) => r,
        Err(_) => (input, input),
    };
    let frag = *cursor.fragment();

    if frag.starts_with("-n") {
        let after_flag = match advance_str(cursor, "-n".len()) {
            Some(a) => a,
            None => return (cursor, None),
        };
        let (after_ws, _) = match space1(after_flag) {
            Ok(r) => r,
            Err(_) => return (cursor, None),
        };
        match take_name_with_macros(state, after_ws) {
            Some((after_name, name)) => (after_name, Some(SubpkgRef::Absolute(name))),
            None => (cursor, None),
        }
    } else if frag.is_empty() || frag.starts_with('\n') || frag.starts_with('\r') {
        // No args at all — e.g. `%description` for the main package.
        (cursor, None)
    } else {
        match take_name_with_macros(state, cursor) {
            Some((after_name, name)) => (after_name, Some(SubpkgRef::Relative(name))),
            None => (cursor, None),
        }
    }
}

/// Parse a section name argument that may contain macro references like
/// `%{shortname}-sub1`. Stops at whitespace, EOL, or EOF. Returns `None`
/// when the cursor sits on a terminator (no name to consume).
pub(crate) fn take_name_with_macros<'a>(
    state: &ParserState,
    input: Input<'a>,
) -> Option<(Input<'a>, Text)> {
    let frag = *input.fragment();
    let first = frag.chars().next()?;
    if matches!(first, ' ' | '\t' | '\n' | '\r') {
        return None;
    }
    let is_terminator = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    match parse_text(state, input, &is_terminator) {
        Ok((rest, text)) => {
            if text.segments.is_empty() {
                None
            } else {
                Some((rest, text))
            }
        }
        Err(_) => None,
    }
}

fn advance_str<'a>(input: Input<'a>, n: usize) -> Option<Input<'a>> {
    if input.fragment().len() < n {
        return None;
    }
    let (rest, _) = nom::Input::take_split(&input, n);
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{CondKind, PreambleContent, Tag};

    fn parse(src: &str) -> Section<Span> {
        let state = ParserState::new();
        let inp = Input::new(src);
        let (_rest, sec) = parse_section(&state, inp).unwrap();
        sec.expect("section recognized")
    }

    #[test]
    fn description_main() {
        let s = parse("%description\nLine one.\nLine two.\n");
        match s {
            Section::Description { subpkg, body, .. } => {
                assert!(subpkg.is_none());
                assert_eq!(body.lines.len(), 2);
                assert_eq!(body.lines[0].literal_str(), Some("Line one."));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_relative() {
        let s = parse("%description foo\nText body.\n");
        match s {
            Section::Description { subpkg, body, .. } => {
                match subpkg.unwrap() {
                    SubpkgRef::Relative(t) => assert_eq!(t.literal_str(), Some("foo")),
                    _ => panic!(),
                }
                assert_eq!(body.lines.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_absolute() {
        let s = parse("%description -n libfoo\nhi\n");
        match s {
            Section::Description { subpkg, .. } => match subpkg.unwrap() {
                SubpkgRef::Absolute(t) => assert_eq!(t.literal_str(), Some("libfoo")),
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn description_subpkg_with_macro_suffix() {
        // Regression: real-world specs like texlive-base.spec use
        // `%description -n %{shortname}-sub1`. The header argument must
        // accept macro segments, not only literal identifiers.
        let s = parse(
            "%description -n %{shortname}-sub1\nbody one\nbody two\n",
        );
        match s {
            Section::Description { subpkg, body, .. } => {
                match subpkg.expect("subpkg parsed") {
                    SubpkgRef::Absolute(t) => {
                        assert_eq!(t.segments.len(), 2);
                        assert!(matches!(&t.segments[0], TextSegment::Macro(_)));
                        assert!(
                            matches!(&t.segments[1], TextSegment::Literal(s) if s == "-sub1")
                        );
                    }
                    _ => panic!(),
                }
                assert_eq!(body.lines.len(), 2);
                assert_eq!(body.lines[0].literal_str(), Some("body one"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_subpkg_with_macro_suffix() {
        let s = parse("%package -n %{shortname}-sub1\nSummary: x\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                match name_arg {
                    PackageName::Absolute(t) => {
                        assert_eq!(t.segments.len(), 2);
                        assert!(matches!(&t.segments[0], TextSegment::Macro(_)));
                    }
                    _ => panic!(),
                }
                assert_eq!(content.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn description_stops_at_next_section() {
        let s = parse("%description\nbody1\nbody2\n%files\n/path\n");
        match s {
            Section::Description { body, .. } => {
                assert_eq!(body.lines.len(), 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_with_preamble() {
        let s = parse("%package foo\nSummary: Foo subpkg\nRequires: bar\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                match name_arg {
                    PackageName::Relative(t) => assert_eq!(t.literal_str(), Some("foo")),
                    _ => panic!(),
                }
                assert_eq!(content.len(), 2);
                match &content[0] {
                    PreambleContent::Item(p) => assert!(matches!(p.tag, Tag::Summary)),
                    other => panic!("{other:?}"),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_absolute_name() {
        let s = parse("%package -n libfoo\nLicense: MIT\n");
        match s {
            Section::Package {
                name_arg, content, ..
            } => {
                assert!(matches!(name_arg, PackageName::Absolute(_)));
                assert_eq!(content.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn package_body_with_comment_and_blank() {
        let s = parse("%package foo\n# a comment\n\nSummary: X\n");
        match s {
            Section::Package { content, .. } => {
                assert_eq!(content.len(), 3);
                assert!(matches!(content[0], PreambleContent::Comment(_)));
                assert!(matches!(content[1], PreambleContent::Blank));
                assert!(matches!(content[2], PreambleContent::Item(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn peek_section_returns_some_for_known() {
        let inp = Input::new("%description hi\n");
        assert_eq!(peek_section_header(inp), Some("%description"));
    }

    #[test]
    fn peek_section_returns_none_for_other() {
        let inp = Input::new("Name: hello\n");
        assert!(peek_section_header(inp).is_none());
    }

    // ---- shell-body structural conditionals ---------------------------

    fn parse_build_install(src: &str) -> ShellBody<Span> {
        let s = parse(src);
        match s {
            Section::BuildScript { body, .. } => body,
            other => panic!("expected %install scriptlet, got {other:?}"),
        }
    }

    #[test]
    fn shell_body_flat_no_conditionals_when_no_if() {
        let body = parse_build_install("%install\necho hi\nmake install\n");
        assert!(body.conditionals.is_empty());
        assert_eq!(body.lines.len(), 2);
    }

    #[test]
    fn shell_body_detects_simple_if_endif() {
        let body = parse_build_install(
            "%install\n%if 0\nhello\n%endif\n",
        );
        // Lines view still includes %if / %endif as raw text.
        assert_eq!(body.lines.len(), 3);
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches.len(), 1);
        assert_eq!(c.branches[0].head_line, 2); // `%install` on line 1
        assert!(c.otherwise.is_none());
    }

    #[test]
    fn shell_body_detects_if_else_endif() {
        let body = parse_build_install(
            "%install\n%if 1\nbody1\n%else\nbody2\n%endif\n",
        );
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches.len(), 1);
        assert_eq!(c.branches[0].head_line, 2);
        let els = c.otherwise.as_ref().expect("else present");
        assert_eq!(els.head_line, 4);
    }

    #[test]
    fn shell_body_detects_nested_conditionals_flat() {
        // Nested %if blocks: emit BOTH as separate flat entries so each gets
        // its own tag in `matrix expand`. Source-order sort puts outer first.
        let body = parse_build_install(
            "%install\n%if 1\n  %if 2\n  inner\n  %endif\n%endif\n",
        );
        assert_eq!(body.conditionals.len(), 2);
        // First entry by start_byte is the outer (line 2); inner starts later (line 3).
        assert_eq!(body.conditionals[0].branches[0].head_line, 2);
        assert_eq!(body.conditionals[1].branches[0].head_line, 3);
        // Outer span contains the inner span.
        let outer = &body.conditionals[0].data;
        let inner = &body.conditionals[1].data;
        assert!(outer.start_byte <= inner.start_byte);
        assert!(outer.end_byte >= inner.end_byte);
    }

    #[test]
    fn shell_body_rewinds_when_section_header_appears_inside_if() {
        // Real-world rpm idiom: `%if` wraps subsequent sections, not the
        // scriptlet body. Previously the shell-body collector would
        // greedily swallow `%if`, hit the next `%post` header with the
        // `%if` still open, and emit a spurious "unterminated `%if`"
        // error. The rewind anchor must hand the `%if` back to the
        // spec-level parser, leaving the scriptlet body intact.
        let src = "%postun server\n\
                   /sbin/ldconfig\n\
                   \n\
                   %if %plperl\n\
                   %post -p /sbin/ldconfig plperl\n\
                   %endif\n";
        let state = ParserState::new();
        let inp = Input::new(src);
        let (rest, sec_opt) = parse_section(&state, inp).unwrap();
        let sec = sec_opt.expect("section recognized");
        // The `%postun server` scriptlet body must contain only the
        // `/sbin/ldconfig` line (trailing blank trimmed). The `%if`
        // and everything after it stay in `rest` for the next
        // top-level parse iteration.
        match sec {
            Section::Scriptlet(scr) => {
                let body = &scr.body;
                assert_eq!(
                    body.lines.len(),
                    1,
                    "scriptlet body should stop before `%if`, got {:?}",
                    body.lines.iter().map(|t| t.literal_str().unwrap_or("?")).collect::<Vec<_>>()
                );
                assert!(body.conditionals.is_empty(), "`%if` belongs to spec level, not the body");
            }
            other => panic!("expected scriptlet section, got {other:?}"),
        }
        assert!(
            rest.fragment().starts_with("%if %plperl"),
            "rest should resume at the rewound `%if`, got: {:?}",
            &rest.fragment().chars().take(40).collect::<String>()
        );
        // And no unterminated-conditional error should have been emitted.
        let diags = state.snapshot_diagnostics();
        let errs: Vec<&str> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some(codes::E_UNTERMINATED_CONDITIONAL))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            errs.is_empty(),
            "no unterminated-`%if` error should fire; got {errs:?}"
        );
    }

    #[test]
    fn shell_body_preserves_properly_closed_if_then_section() {
        // Sanity check the rewind doesn't break the common case where
        // `%if`/`%endif` is fully contained within the scriptlet AND a
        // section header follows.
        let src = "%post\n\
                   %if 0%{?something}\n\
                   echo conditional\n\
                   %endif\n\
                   echo always\n\
                   %postun\n\
                   /sbin/ldconfig\n";
        let state = ParserState::new();
        let inp = Input::new(src);
        let (rest, sec_opt) = parse_section(&state, inp).unwrap();
        let sec = sec_opt.expect("section recognized");
        match sec {
            Section::Scriptlet(scr) => {
                let body = &scr.body;
                // Body should include the %if/echo/%endif/echo lines —
                // four lines (blank trimmed because there's no trailing
                // blank between body and `%postun`).
                assert!(
                    body.lines.len() >= 4,
                    "scriptlet body should retain all four lines, got {:?}",
                    body.lines.iter().map(|t| t.literal_str().unwrap_or("?")).collect::<Vec<_>>()
                );
                assert_eq!(body.conditionals.len(), 1, "properly closed `%if` stays shell-level");
            }
            other => panic!("expected scriptlet section, got {other:?}"),
        }
        assert!(
            rest.fragment().starts_with("%postun"),
            "rest should start at the next section header"
        );
        let diags = state.snapshot_diagnostics();
        let errs: Vec<&str> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some(codes::E_UNTERMINATED_CONDITIONAL))
            .map(|d| d.message.as_str())
            .collect();
        assert!(errs.is_empty(), "no spurious unterminated-`%if` error; got {errs:?}");
    }

    #[test]
    fn shell_body_detects_elif_chain() {
        let body = parse_build_install(
            "%install\n%if 1\na\n%elif 2\nb\n%elif 3\nc\n%else\nd\n%endif\n",
        );
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches.len(), 3);
        assert_eq!(c.branches[0].kind, CondKind::If);
        assert_eq!(c.branches[1].kind, CondKind::Elif);
        assert_eq!(c.branches[2].kind, CondKind::Elif);
        assert!(c.otherwise.is_some());
        // Verify the `%elif 2` head was parsed as a structured integer literal.
        use crate::ast::{CondExpr, ExprAst};
        match &c.branches[1].expr {
            CondExpr::Parsed(boxed) => match boxed.as_ref() {
                ExprAst::Integer { value, .. } => assert_eq!(*value, 2),
                other => panic!("expected Parsed Integer 2, got {other:?}"),
            },
            other => panic!("expected Parsed expr, got {other:?}"),
        }
    }

    #[test]
    fn shell_body_detects_ifarch() {
        let body = parse_build_install(
            "%install\n%ifarch x86_64 aarch64\nhello\n%endif\n",
        );
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches[0].kind, CondKind::IfArch);
    }

    /// Extract the first `Section::BuildScript`'s body from a parsed spec.
    fn first_build_script_body(spec: &crate::ast::SpecFile<Span>) -> &ShellBody<Span> {
        for item in &spec.items {
            if let crate::ast::SpecItem::Section(boxed) = item {
                if let Section::BuildScript { body, .. } = boxed.as_ref() {
                    return body;
                }
            }
        }
        panic!("no BuildScript section found");
    }

    #[test]
    fn shell_body_unterminated_if_recovers() {
        // No `%endif`: parser should still surface partial ShellConditional
        // and emit an E_UNTERMINATED_CONDITIONAL diagnostic.
        let src = "%install\n%if 1\nhello\n";
        let result = crate::parser::parse_str_with_spans(src);
        let body = first_build_script_body(&result.spec);
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches.len(), 1);
        // End extends to end of source so analyses still see the partial structure.
        assert!(c.data.end_byte >= src.len() - 1);
        // Diagnostic emitted with the correct code.
        assert!(
            result.diagnostics.iter().any(|d| d.code.as_deref()
                == Some(crate::parse_result::codes::E_UNTERMINATED_CONDITIONAL)),
            "expected E_UNTERMINATED_CONDITIONAL, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn shell_body_detects_if_with_macro() {
        let body = parse_build_install(
            "%install\n%if %{with bootstrap}\nbody\n%endif\n",
        );
        assert_eq!(body.conditionals.len(), 1);
        let c = &body.conditionals[0];
        assert_eq!(c.branches.len(), 1);
        assert_eq!(c.branches[0].kind, CondKind::If);
        // Should be Parsed (not Raw fallback). %{with foo} is a macro reference.
        use crate::ast::CondExpr;
        assert!(
            matches!(&c.branches[0].expr, CondExpr::Parsed(_)),
            "expected Parsed expr, got: {:?}",
            c.branches[0].expr
        );
    }

    #[test]
    fn shell_body_repeated_else_warns() {
        let src = "%install\n%if 1\na\n%else\nb\n%else\nc\n%endif\n";
        let result = crate::parser::parse_str_with_spans(src);
        assert!(
            result.diagnostics.iter().any(|d| d.code.as_deref()
                == Some(crate::parse_result::codes::W_MULTIPLE_ELSE)),
            "expected W_MULTIPLE_ELSE, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn shell_body_depth_limit_warns_once() {
        // Generate 70 nested %if without endif (exceeds MAX_SHELL_COND_DEPTH=64).
        let mut src = String::from("%install\n");
        for _ in 0..70 {
            src.push_str("%if 1\n");
        }
        src.push_str("body\n");
        let result = crate::parser::parse_str_with_spans(&src);
        // Exactly one depth-limit warning should be emitted (rate-limited at first hit).
        let depth_warnings = result
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("MAX_SHELL_COND_DEPTH"))
            .count();
        assert_eq!(
            depth_warnings, 1,
            "depth-limit warning should fire exactly once, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn shell_body_stray_endif_diagnosed() {
        let src = "%install\n%endif\necho hi\n";
        let result = crate::parser::parse_str_with_spans(src);
        assert!(
            result.diagnostics.iter().any(|d| {
                d.code.as_deref() == Some(crate::parse_result::codes::E_UNTERMINATED_CONDITIONAL)
                    && d.message.contains("without matching")
            }),
            "expected stray %endif diagnostic, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn shell_body_elif_after_else_diagnosed() {
        let src = "%install\n%if 1\na\n%else\nb\n%elif 2\nc\n%endif\n";
        let result = crate::parser::parse_str_with_spans(src);
        assert!(
            result.diagnostics.iter().any(|d| d.code.as_deref()
                == Some(crate::parse_result::codes::W_ELIF_AFTER_ELSE)),
            "expected W_ELIF_AFTER_ELSE, got: {:?}",
            result.diagnostics
        );
    }
}
