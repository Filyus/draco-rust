# Repository Instructions

## Rust Formatting

Rust code follows the Chromium Rust Style Guide, which currently relies on the
public Rust Style Guide for mechanical formatting and the Rust API Guidelines
for API design.

Use stable `rustfmt` for all Rust formatting. Before finalizing Rust changes,
run:

```sh
cargo fmt --manifest-path crates/Cargo.toml --all -- --check
cargo fmt --manifest-path web/Cargo.toml --all -- --check
```

To format the workspaces, run the same commands without `-- --check`.
