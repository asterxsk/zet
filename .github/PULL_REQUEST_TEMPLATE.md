## What this changes

<!-- One paragraph. What is different after this than before it, in the terms the changelog entry
     will use. -->

Closes #

## Why

<!-- The constraint the code cannot show, or the reason the obvious approach was not taken. If this
     contradicts DESIGN.md or PRODUCT.md, say so and say why — that is the part of the review that
     matters most. -->

## How it was checked

<!-- What you ran, and what you did by hand. "Tests pass" is the floor, not the answer: name the
     test that fails against the old behaviour, or describe the app being driven. -->

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Checklist

- [ ] An issue is behind it, for anything that changes behaviour, configuration, or a keybinding.
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo test --workspace` all pass locally.
- [ ] A test covers the new behaviour and fails without the fix.
- [ ] An entry under `## [Unreleased]` in [CHANGELOG.md](../CHANGELOG.md), if a user would notice.
- [ ] Comments say why, not what.
- [ ] No crate reaches upward past its place in [Cargo.toml](../Cargo.toml)'s dependency order.
