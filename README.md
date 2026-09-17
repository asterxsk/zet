# zet

A terminal multiplexer for Windows, written in Rust, with its own VT engine and a GPU
renderer.

> **Status: pre-alpha.** The terminal engine and the process layer work and are tested. There
> is no window to run them in yet — the renderer, the configuration, and the binary are still
> to come. Nothing here is usable as a daily driver, and nothing here is worth installing.

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

| Crate | What it owns |
|---|---|
| [`zet-vt`](crates/zet-vt) | The terminal engine: parser, grid, scrollback, reflow, damage |
| [`zet-pty`](crates/zet-pty) | ConPTY sessions and shell profile discovery |
| [`zet-update`](crates/zet-update) | Version identity, update checks, self-replacement |
| `zet-render` | The wgpu renderer on DirectX 12 — planned |
| `zet-config` | TOML configuration, themes, hot reload — planned |
| `zet` | The binary — planned |

## Building

Requires Rust 1.90 or newer and the MSVC toolchain.

```sh
cargo build --release
cargo test --workspace
```

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
- **Breaking.** Any change to [`zet-update`](crates/zet-update)'s public API, which is published
  as a library.

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

zet checks GitHub Releases on launch and tells you when there is a newer version. You choose
whether to install it, and it takes effect the next time zet starts.

The archive is verified against a `SHA256SUMS` digest published alongside it before anything is
written to disk. **The archives are not Authenticode-signed**, so a checksum proves the download
matches the release — not who published it. What that does and does not buy you is set out in
[SECURITY.md](SECURITY.md).

To turn the check off and make zet never touch the network on its own:

```toml
[update]
check_on_launch = false
```

## Privacy

zet has no telemetry, no accounts, and no server. The update check is the only request it makes
on its own, and it discloses your IP address to GitHub. [PRIVACY.md](PRIVACY.md) says exactly
what is sent, what is not, and what is written to your disk.

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
