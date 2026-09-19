# zet

A terminal multiplexer for Windows, written in Rust, with its own VT engine and a GPU
renderer.

> **Status: pre-alpha.** zet builds and runs: it opens a window, starts a shell, and draws it
> with its own VT engine and its own GPU renderer. It speaks the kitty keyboard protocol in
> both directions — the flag stack, the query, and the `CSI u` encoding, with `Report alternate
> keys` the one flag it does not implement. What is missing is the depth around that:
> the update check exists as `zet --check-update` and `zet --update`, and never runs on launch —
> nothing is planned to change that.
> It installs and runs as a daily driver for the person who wrote it, and pre-alpha is the
> honest label for what that is worth to anyone else: expect the depth around the terminal to
> move, and expect the configuration file to change under you.

## Why

Windows terminals each make you give something up. Windows Terminal is good but its rendering
and configuration are Microsoft's to decide. Alacritty is fast but has no tabs and no settings
UI. WezTerm has everything and needs its own manual. None of them let you own the part that
matters.

There is also a correctness problem that only shows up when you live in a terminal. Full-screen
TUI programs flicker on every repaint unless the terminal honors synchronized output.
Alternate-screen apps leave artifacts on resize. Mouse reporting, truecolor, and hyperlinks work
in some terminals and half-work in others. You notice the first time you run an agentic coding
tool inside one.

zet owns its terminal engine end to end: the byte parser, the screen grid, the scrollback, the
reflow, the damage tracking, and the renderer. Nothing about the character pipeline is borrowed.
That is the point of the project, and the reason the rest of it exists.

The full product thinking is in [PRODUCT.md](PRODUCT.md); the design decisions are in
[DESIGN.md](DESIGN.md).

## The crates

The workspace is layered, and each crate is built on the ones above it in this list — except
[`zet-update`](crates/zet-update), which stands alone and is the only crate here structured as a
library in its own right. None of them is on crates.io.

| Crate | What it owns |
|---|---|
| [`zet-vt`](crates/zet-vt) | The terminal engine: parser, grid, scrollback, reflow, damage |
| [`zet-pty`](crates/zet-pty) | ConPTY sessions and shell profile discovery |
| [`zet-config`](crates/zet-config) | TOML configuration, themes, the ANSI palette, hot reload |
| [`zet-font`](crates/zet-font) | Font loading, rasterisation, the face chain and its fallback order |
| [`zet-input`](crates/zet-input) | Key and mouse encoding, chords and keybindings |
| [`zet-session`](crates/zet-session) | A session: a pty and a terminal with a read loop between them |
| [`zet-render`](crates/zet-render) | The wgpu renderer: the glyph atlas, the grid and chrome pipelines |
| [`zet-ui`](crates/zet-ui) | The chrome's layout, and the embedded IBM Plex Sans it is drawn in |
| [`zet-app`](crates/zet-app) | The application state machine: tabs, commands, themes, selection |
| [`zet`](crates/zet) | The binary: the window, the event loop, and the frame host |
| [`zet-update`](crates/zet-update) | Version identity, update checks, self-replacement |

## Building

Requires Rust 1.90 or newer and the MSVC toolchain.

```sh
cargo build --release
cargo test --workspace
```

That produces `target/release/zet.exe`, which is the whole program — it needs no assets beside
it, because the chrome's fonts are compiled in. It looks for its configuration at
`%APPDATA%\zet\config.toml` and starts with defaults if the file is not there.

The tests that talk to GitHub are ignored by default, because a test that needs the network is a
test that fails on a train:

```sh
cargo test -p zet-update --test live -- --ignored --nocapture
```

## Versioning

zet follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html), with the usual
pre-1.0 caveat: while the major version is `0`, a **minor** bump may break compatibility and a
**patch** bump will not. `0.2.0` to `0.3.0` can break your configuration; `0.3.0` to `0.3.1`
cannot.

What "breaking" means for a terminal is narrower than it sounds, because most of what a
terminal emits is not zet's API:

- **Not breaking.** Anything a program inside zet sees. Escape sequences, the alternate screen,
  mouse reporting, bracketed paste, and the rest are standards; they change when the standards
  change, not when zet does. A TUI that worked in `0.3.0` works in `0.4.0`.
- **Not breaking.** Performance, rendering quality, and bug fixes.
- **Breaking.** A keybinding that stops working or starts doing something else.
- **Breaking.** A configuration key that is renamed, moved, or removed, or whose default
  changes in a way you would notice.
- **Breaking.** A theme file format change.
- **Breaking.** Any change to [`zet-update`](crates/zet-update)'s public API, which is written as
  a library even though it is not published as one.

Every release is listed in [CHANGELOG.md](CHANGELOG.md), which is the authoritative record of
what changed and which section of it is breaking.

### Release process

Releases are tags. There is no release branch and no manual build step:

```sh
# 1. Move the version in Cargo.toml, under [workspace.package].
# 2. Move the entries in CHANGELOG.md out of "Unreleased" into the new version.
git commit -am "Release 0.2.0"
git tag v0.2.0
git push origin main v0.2.0
```

[`.github/workflows/release.yml`](.github/workflows/release.yml) then verifies that the tag and
`Cargo.toml` agree, builds the binary, writes `SHA256SUMS`, and publishes a GitHub Release. The
tag and the manifest are checked against each other because a binary that reports a different
version from the tag it shipped under makes the updater offer the same release on every launch,
forever.

Tags are `v`-prefixed. Both `v0.2.0` and `0.2.0` parse, but the workflow creates the former.

## Updates

Two commands, both of them requests you type:

```sh
zet --check-update    # is there a newer version?
zet --update          # if there is, install it
```

`--check-update` prints the answer and exits: it downloads nothing and changes nothing on disk.
`--update` is the same check followed by the download, the digest check, and the replacement. The
new version takes effect the next time zet starts, because Windows will not let a running
executable be written to or deleted — but it will let one be renamed, and that is the whole
mechanism.

Neither reads your configuration, and there is nothing to configure: zet makes no request on its
own — not on launch, not on a timer, not in the background. Typing one of these is the only way to
make it ask.

The archive is verified against a `SHA256SUMS` digest published alongside it before anything is
written to disk. **The archives are not Authenticode-signed**, so a checksum proves the download
matches the release — not who published it. What that does and does not buy you is set out in
[SECURITY.md](SECURITY.md).

## Privacy

zet has no telemetry, no accounts, and no server. The only requests it can make are the ones
behind `zet --check-update` and `zet --update`, and they happen only when you type one — nothing
in zet asks on its own, ever. A request discloses your IP address to GitHub and nothing else.
[PRIVACY.md](PRIVACY.md) says exactly what is sent, what is not, and what is written to your disk.

## Security

Report vulnerabilities privately — see [SECURITY.md](SECURITY.md). Do not open a public issue.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the Rust ecosystem's usual arrangement: the MIT terms are the
permissive default, and the Apache terms add an explicit patent grant.

### Contribution

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any
additional terms or conditions.
