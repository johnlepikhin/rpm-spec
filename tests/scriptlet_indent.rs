//! Regression: scriptlet body indentation must be idempotent under
//! `parse → print → parse → print` cycles, even when the scriptlet
//! lives inside a `%if` block and the printer is configured with a
//! non-zero `indent`.

use rpm_spec::parser::parse_str;
use rpm_spec::printer::{PrinterConfig, print_with};

fn pretty(src: &str) -> String {
    let outcome = parse_str(src);
    let cfg = PrinterConfig::default().with_indent(2);
    print_with(&outcome.spec, &cfg)
}

#[test]
fn scriptlet_under_nested_if_is_idempotent() {
    let src = "\
%if !%{with testsuite}
%post
%systemd_post foo.service
%postun
%systemd_postun_with_restart foo.service
%endif
";
    let p1 = pretty(src);
    let p2 = pretty(&p1);
    assert_eq!(p1, p2, "pass1=\n{p1}\npass2=\n{p2}");
}

#[test]
fn nested_if_with_multiple_scriptlets_stable() {
    let src = "\
%if 0
%post
do_a
%pre
do_b
%endif
";
    let p1 = pretty(src);
    let p2 = pretty(&p1);
    let p3 = pretty(&p2);
    assert_eq!(p1, p2);
    assert_eq!(p2, p3);
}
