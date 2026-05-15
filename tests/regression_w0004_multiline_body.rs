//! Regression: `W_UNTERMINATED_MACRO` must not fire on legitimate
//! multi-line `%{?macro:body}` constructs whose body contains
//! section-header-looking lines (`%post`, `%postun`, etc.).
//!
//! Trigger pattern lifted from real-world specs (gcc.spec uses this
//! idiom for conditional ldconfig scriptlet wrapping):
//!
//! ```spec
//! %{?ldconfig:
//! %post -n libgcc -p <lua>
//! ...
//! %postun -n libgcc -p <lua>
//! ...
//! }
//! ```
//!
//! `parse_text` cannot follow the body across the `%post` line
//! (it tries to recursively parse `%post` as a macro reference and
//! the scan never finds the closing `}`), but `rpm` itself handles
//! this correctly. The spurious warning is dropped at the
//! `parse_body_as_text` boundary — its span would be body-relative
//! and point at an unrelated line in the file.

use rpm_spec::parse_result::codes;
use rpm_spec::parser::parse_str;

const TRIGGER: &str = "\
Name:    x
Version: 1
Release: 1
Summary: s
License: MIT

%{?ldconfig:
%post -n libgcc -p <lua>
if posix.access(\"%ldconfig\", \"x\") then
  rpm.execute(\"%ldconfig\")
end

%postun -n libgcc -p <lua>
if posix.access(\"%ldconfig\", \"x\") then
  rpm.execute(\"%ldconfig\")
end
}

%description
b

%changelog
* Mon Jan 01 2024 a <a@b> - 1-1
- init
";

#[test]
fn no_unterminated_macro_warning_on_multiline_conditional_body() {
    let outcome = parse_str(TRIGGER);
    let unterminated = outcome
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some(codes::W_UNTERMINATED_MACRO))
        .count();
    assert_eq!(
        unterminated, 0,
        "expected zero W_UNTERMINATED_MACRO; got diagnostics: {:?}",
        outcome.diagnostics
    );
}
