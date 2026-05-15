//! Shared printer helpers used by multiple sub-modules.

use crate::ast::SubpkgRef;

use super::Printer;
use super::text::print_text;

/// Render the optional `[-n NAME]` / bare-name modifier that appears
/// after most section headers (`%description`, `%files`, `%post`,
/// `%triggerin`, etc.). Renders nothing when `subpkg` is `None`.
pub(crate) fn print_subpkg(p: &mut Printer<'_>, subpkg: Option<&SubpkgRef>) {
    match subpkg {
        Some(SubpkgRef::Absolute(name)) => {
            p.raw(" -n ");
            print_text(p, name);
        }
        Some(SubpkgRef::Relative(name)) => {
            p.raw_char(' ');
            print_text(p, name);
        }
        None => {}
    }
}
