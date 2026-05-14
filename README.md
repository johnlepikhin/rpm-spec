# rpm-spec

A distribution-independent parser and pretty-printer for RPM `.spec` files,
written in Rust.

## Status

**Pre-alpha.** The AST is in place. The parser and printer are stubs.

A future companion crate, `rpm-spec-validator`, will layer
distribution-specific macro registries (Fedora, RHEL, openSUSE, Mageia) on
top of this AST.

## Workspace layout

- `crates/rpm-spec` — AST, parser, printer.

## License

MIT OR Apache-2.0.
