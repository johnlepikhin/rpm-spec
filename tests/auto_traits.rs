//! Compile-time proofs that the public types are `Send + Sync`. The
//! body of every test is `()` — the assertions are encoded in the
//! generic bounds and resolve at type-check time.

use rpm_spec::ast::{Span, SpecFile};
use rpm_spec::error::{ParseError, PrintError};
use rpm_spec::parse_result::{Diagnostic, ParseResult, Severity};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn ast_root_is_send_sync() {
    assert_send_sync::<SpecFile<()>>();
    assert_send_sync::<SpecFile<Span>>();
}

#[test]
fn parse_result_is_send_sync() {
    assert_send_sync::<ParseResult<()>>();
    assert_send_sync::<ParseResult<Span>>();
}

#[test]
fn diagnostics_are_send_sync() {
    assert_send_sync::<Diagnostic>();
    assert_send_sync::<Severity>();
}

#[test]
fn error_types_are_send_sync() {
    assert_send_sync::<ParseError>();
    assert_send_sync::<PrintError>();
}

#[test]
fn span_is_send_sync_copy() {
    assert_send_sync::<Span>();
    fn assert_copy<T: Copy>() {}
    assert_copy::<Span>();
}
