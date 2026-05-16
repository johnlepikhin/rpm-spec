//! Integration tests for stage 3 of the parser: section bodies
//! (`%prep`/`%build`/`%install`/`%check`/`%files`/`%changelog`/scriptlets/
//! triggers/`%verify`/`%sourcelist`).

use rpm_spec::ast::{
    BuildScriptKind, ConfigFlag, FileDirective, FileTrigger, FileTriggerKind, FilesContent,
    Interpreter, Month, PackageName, ScriptletKind, Section, SpecItem, SubpkgRef, Trigger,
    TriggerKind, Weekday,
};
use rpm_spec::parser::parse_str;

const FULL_SPEC: &str = "\
Name:           hello
Version:        1.0
Release:        1%{?dist}
Summary:        Greets the world
License:        MIT
URL:            https://example.org/hello
BuildArch:      noarch
Source0:        hello-%{version}.tar.gz

BuildRequires:  gcc make
Requires:       glibc

%description
The hello package greets the world.

%package -n libhello
Summary:        Greeter library
License:        MIT

%description -n libhello
Library half of hello.

%prep
%autosetup -p1

%build
%configure
%make_build

%install
%make_install
rm -f %{buildroot}/usr/lib/*.la

%check
make check

%files
%license LICENSE
%doc README.md
%attr(0755,root,root) /usr/bin/hello
%config(noreplace) /etc/hello.conf

%files -n libhello
%{_libdir}/libhello.so.*

%post -p /sbin/ldconfig

%post libhello -p /sbin/ldconfig

%triggerin -- foo, bar >= 1.0
echo trigger fired

%filetriggerin -P 200 -- /usr/lib
do-something

%changelog
* Wed May 14 2025 Maintainer <m@example.org> - 1.0-1
- initial packaging
";

#[test]
fn full_spec_parses_without_deferred_diagnostics() {
    let r = parse_str(FULL_SPEC);
    let deferred: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not yet implemented"))
        .collect();
    assert!(deferred.is_empty(), "{deferred:?}");
}

#[test]
fn build_script_sections_present() {
    let r = parse_str(FULL_SPEC);
    let kinds: Vec<BuildScriptKind> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::BuildScript { kind, .. } => Some(*kind),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(kinds.contains(&BuildScriptKind::Prep));
    assert!(kinds.contains(&BuildScriptKind::Build));
    assert!(kinds.contains(&BuildScriptKind::Install));
    assert!(kinds.contains(&BuildScriptKind::Check));
}

#[test]
fn install_body_has_macros_and_literal_lines() {
    let r = parse_str(FULL_SPEC);
    let install_body = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::BuildScript {
                    kind: BuildScriptKind::Install,
                    body,
                    ..
                } => Some(body),
                _ => None,
            },
            _ => None,
        })
        .expect("%install");
    assert_eq!(install_body.lines.len(), 2);
    let second = &install_body.lines[1];
    // rm -f %{buildroot}/usr/lib/*.la — must include a macro segment.
    assert!(
        second
            .segments
            .iter()
            .any(|s| matches!(s, rpm_spec::ast::TextSegment::Macro(m) if m.name == "buildroot"))
    );
}

#[test]
fn files_section_directives_parsed() {
    let r = parse_str(FULL_SPEC);
    let main_files = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Files {
                    subpkg: None,
                    content,
                    ..
                } => Some(content),
                _ => None,
            },
            _ => None,
        })
        .expect("main %files");

    let entry_kinds: Vec<Vec<&'static str>> = main_files
        .iter()
        .filter_map(|c| match c {
            FilesContent::Entry(e) => Some(
                e.directives
                    .iter()
                    .map(|d| match d {
                        FileDirective::License => "license",
                        FileDirective::Doc => "doc",
                        FileDirective::Attr(_) => "attr",
                        FileDirective::Config(_) => "config",
                        _ => "other",
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    assert!(entry_kinds.iter().any(|v| v == &["license"]));
    assert!(entry_kinds.iter().any(|v| v == &["doc"]));
    assert!(entry_kinds.iter().any(|v| v == &["attr"]));
    assert!(entry_kinds.iter().any(|v| v == &["config"]));
}

#[test]
fn config_noreplace_flag() {
    let r = parse_str(FULL_SPEC);
    let main_files = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Files {
                    subpkg: None,
                    content,
                    ..
                } => Some(content),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let cfg = main_files
        .iter()
        .find_map(|c| match c {
            FilesContent::Entry(e) => e.directives.iter().find_map(|d| match d {
                FileDirective::Config(flags) => Some(flags.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap();
    assert_eq!(cfg, vec![ConfigFlag::NoReplace]);
}

#[test]
fn libhello_subpackage_with_two_files_sections() {
    let r = parse_str(FULL_SPEC);
    let libhello_files = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Files {
                    subpkg: Some(SubpkgRef::Absolute(t)),
                    content,
                    ..
                } if t.literal_str() == Some("libhello") => Some(content),
                _ => None,
            },
            _ => None,
        })
        .next()
        .expect("%files -n libhello");
    assert!(!libhello_files.is_empty());
}

#[test]
fn package_main_and_sub_present() {
    let r = parse_str(FULL_SPEC);
    let packages: Vec<&PackageName> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Package { name_arg, .. } => Some(name_arg),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(packages.len(), 1);
    assert!(matches!(packages[0], PackageName::Absolute(_)));
}

#[test]
fn scriptlets_parsed() {
    let r = parse_str(FULL_SPEC);
    let scriptlets: Vec<_> = r
        .spec
        .items
        .iter()
        .filter_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Scriptlet(sc) => Some(sc),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(scriptlets.len(), 2);
    // Both are %post.
    assert!(
        scriptlets
            .iter()
            .all(|sc| matches!(sc.kind, ScriptletKind::Post))
    );
    // One bare, one with subpkg.
    assert!(scriptlets.iter().any(|sc| sc.subpkg.is_none()));
    assert!(scriptlets.iter().any(|sc| matches!(
        sc.subpkg.as_ref(),
        Some(SubpkgRef::Relative(t)) if t.literal_str() == Some("libhello")
    )));
    // Both use /sbin/ldconfig as interpreter.
    assert!(scriptlets.iter().all(|sc| matches!(
        sc.interp.as_ref(),
        Some(Interpreter::Path(t)) if t.literal_str() == Some("/sbin/ldconfig")
    )));
}

#[test]
fn trigger_with_conditions() {
    let r = parse_str(FULL_SPEC);
    let trigger: &Trigger<_> = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Trigger(t) => Some(t),
                _ => None,
            },
            _ => None,
        })
        .expect("%triggerin");
    assert!(matches!(trigger.kind, TriggerKind::In));
    assert_eq!(trigger.conditions.len(), 2);
    assert_eq!(trigger.body.lines.len(), 1);
}

#[test]
fn file_trigger_with_priority() {
    let r = parse_str(FULL_SPEC);
    let ft: &FileTrigger<_> = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::FileTrigger(ft) => Some(ft),
                _ => None,
            },
            _ => None,
        })
        .expect("%filetriggerin");
    assert!(matches!(ft.kind, FileTriggerKind::In));
    assert_eq!(ft.priority, Some(200));
    assert_eq!(ft.prefixes.len(), 1);
    assert_eq!(ft.prefixes[0].literal_str(), Some("/usr/lib"));
}

#[test]
fn changelog_one_entry() {
    let r = parse_str(FULL_SPEC);
    let entries = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Changelog { entries, .. } => Some(entries),
                _ => None,
            },
            _ => None,
        })
        .expect("%changelog");
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.date.year, 2025);
    assert_eq!(e.date.month, Month::May);
    assert_eq!(e.date.weekday, Weekday::Wed);
    assert_eq!(e.author.literal_str(), Some("Maintainer"));
    assert_eq!(
        e.email.as_ref().unwrap().literal_str(),
        Some("m@example.org")
    );
    assert_eq!(e.version.as_ref().unwrap().literal_str(), Some("1.0-1"));
}

const VERIFY_SPEC: &str = "\
Name: t\nVersion: 1\nRelease: 1\nSummary: x\nLicense: MIT\n\n%description\nx\n\n%verify\n[ -x /usr/bin/t ]\n";

#[test]
fn verify_section_parsed() {
    let r = parse_str(VERIFY_SPEC);
    let v_body = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Verify { body, .. } => Some(body),
                _ => None,
            },
            _ => None,
        })
        .expect("%verify");
    assert_eq!(v_body.lines.len(), 1);
}

const SOURCELIST_SPEC: &str = "\
Name: t\nVersion: 1\nRelease: 1\nSummary: x\nLicense: MIT\n\n%sourcelist\nhttps://example.org/t-1.tar.gz\nhttps://example.org/t-1.sig\n\n%description\nx\n";

#[test]
fn sourcelist_section_parsed() {
    let r = parse_str(SOURCELIST_SPEC);
    let entries = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::SourceList { entries, .. } => Some(entries),
                _ => None,
            },
            _ => None,
        })
        .expect("%sourcelist");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].literal_str(),
        Some("https://example.org/t-1.tar.gz")
    );
}

const FILES_COND_SPEC: &str = "\
Name: t\nVersion: 1\nRelease: 1\nSummary: x\nLicense: MIT\n\n%description\nx\n\n%files\n/usr/bin/always\n%if 0%{?fedora}\n/usr/bin/fed-only\n%endif\n";

#[test]
fn files_conditional_structural() {
    let r = parse_str(FILES_COND_SPEC);
    let content = r
        .spec
        .items
        .iter()
        .find_map(|i| match i {
            SpecItem::Section(s) => match s.as_ref() {
                Section::Files { content, .. } => Some(content),
                _ => None,
            },
            _ => None,
        })
        .expect("%files");
    let cond_count = content
        .iter()
        .filter(|c| matches!(c, FilesContent::Conditional(_)))
        .count();
    assert_eq!(cond_count, 1);
}
