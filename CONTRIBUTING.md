# Contributing to zet

zet is pre-alpha and written by one person. That shapes everything below: issues are welcome and
read, and a pull request is a conversation that starts with an issue rather than a diff.

## Before you write code

Open an issue first. Not as a formality — zet's design is the part that is hard to change later,
and [DESIGN.md](DESIGN.md) settles most questions of how the window behaves and how it is drawn.
A patch that is right in isolation can still be wrong against that document, and finding that out
in review wastes your time.

Small, obviously-correct fixes can skip this: a typo, a crash, a test that is wrong about what the
code does. Anything that changes behaviour, configuration, keybindings, or the chrome should have
an issue behind it.

Feature requests are read but rarely accepted right now. The feature list lives in
[PRODUCT.md](PRODUCT.md) and the reason for the short list is in [README.md](README.md) — the depth
around the terminal is what is being built, not the width.

Security problems do not go in an issue. See [SECURITY.md](SECURITY.md).

## Building

Windows only. Requires the MSVC toolchain and Rust 1.90 or newer.

```sh
cargo build --release
```

That produces `target/release/zet.exe`. There is no separate step: the chrome's fonts and icons
are compiled in, and the configuration file at `%APPDATA%\zet\config.toml` is optional.

A debug build works the same way and is what you want while iterating:

```sh
cargo run
```

## The four commands CI runs

CI is the contract, and it is short. Run all four before opening a pull request:

```sh
cargo fmt --all --check
```

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

```sh
cargo test --workspace
```

The clippy job sets `RUSTFLAGS: -D warnings`, so a warning fails the build locally too. A warning
that nobody has to fix is a warning nobody reads.

The tests that talk to GitHub are `#[ignore]`d so a flaky network is never the reason a pull
request looks broken. Nothing runs them automatically — run them on purpose when your change
touches the update path:

```sh
cargo test -p zet-update --test live -- --ignored --nocapture
```

There is also a latency benchmark, measured on every push rather than assumed, because
[PRODUCT.md](PRODUCT.md) promises numbers:

```sh
cargo bench -p zet-app --bench latency
```

## What the code should look like

`rustfmt` and `clippy` decide the mechanics. Three things they cannot decide:

- **A comment says why, not what.** This codebase is comment-dense on purpose: the comments name
  the constraint the code cannot show — an ordering the platform imposes, a case that was tried and
  failed, why this seam and not the obvious one. A comment that narrates the next line is noise.
- **The layering is the design.** The workspace members in
  [Cargo.toml](Cargo.toml) are listed in dependency order, and nothing above may be used by
  anything below. If your change needs a crate to reach upward, that is the discussion to have in
  the issue.
- **A behaviour change comes with a test.** Not a coverage exercise — a test that fails against the
  old behaviour and would have caught the bug you are fixing.

Commit subjects are a sentence in the imperative describing the whole change, the way the existing
history reads ("Make the overview's intro and status one band"). No `feat:`/`fix:` prefixes. The
body, when there is one, says why.

The one marker the history uses is `[skip ci]`, and only on a commit that cannot change a build:
documentation, repository housekeeping, a README. Anything that touches `crates/`,
`packaging/`, or a workflow runs CI.

## Changelog

Every user-visible change gets an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md),
in the section that fits it, in the style the existing entries use: the crate in bold, then what
changed, then the reasoning underneath. The changelog is the authoritative record of what changed
and which part of it is breaking — the release process moves entries out of "Unreleased" into a
version, so an entry that is missing here is missing from the release.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Specs, docs, and the site

Two directories that look similar and are not:

- `specs/` holds design and implementation plans for changes big enough to need one, named
  `YYYY-MM-DD-slug.md`. These are working documents: they record the decisions taken, and later
  what was built and where the build deviated from the plan.
- `docs/` is the published site. It is hand-written HTML with one stylesheet and two fonts, and
  [`.github/workflows/pages.yml`](.github/workflows/pages.yml) publishes the directory exactly as
  it is committed, with no build step. Editing `docs/` is how the site changes.

## Licensing

zet is dual-licensed under MIT or Apache-2.0, at the user's option. Unless you explicitly state
otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the
Apache-2.0 license, is dual-licensed as above, without any additional terms or conditions.

## Conduct

Participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). The short version: argue
about the code, not the person who wrote it.
