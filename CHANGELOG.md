# Changelog

Notable changes to zet, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and zet adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The reasoning behind both, and what
counts as a breaking change in a terminal, is in
[Versioning](README.md#versioning).

## [Unreleased]

Nothing yet. Until 1.0.0 ships, each release is a `0.x` minor and any of them may break
compatibility; see [Versioning](README.md#versioning).

## [0.1.0] — unreleased

The first release, and the first point at which an archive is published. Not yet usable as a
daily driver: there is no window to run it in.

### Added

- **`zet-vt`** — the terminal engine, written from scratch.
  - Cell, row, and grid storage with trailing-blank trimming, soft-wrap tracking, and
    per-row damage.
  - A byte-level VT parser covering the C0/C1 controls, CSI, OSC, DCS, SOS/PM/APC, and the ESC
    intermediates, including the parameter, intermediate, and final dispatch rules.
  - Terminal state: scroll regions, the alternate screen, cursor save/restore, character sets,
    tab stops, insert/replace mode, and origin mode.
  - Colour: 16-colour, 256-colour, and 24-bit SGR, with reverse video resolved at draw time.
  - Reflow across logical lines on resize, and back-colour-erase that respects the current SGR.
  - Synchronized output (`DECSET 2026`) so a full-screen repaint is one frame and not a storm.
- **`zet-pty`** — the process layer.
  - ConPTY sessions: create, resize, write, read, kill, and shut down, with a job object that
    reaps grandchildren the child tried to detach.
  - Shell profile discovery across `PATH` and `PATHEXT`, Program Files, the Windows app-alias
    directory, Git Bash, and WSL distributions read from `wsl.exe -l -v`.
  - A bounded reader channel, so a child that writes faster than the terminal draws feels
    backpressure instead of deadlocking the close.

### Security

- Update checks verify a `SHA256SUMS` digest before an archive is applied. Archives are not
  Authenticode-signed; see [SECURITY.md](SECURITY.md).

[Unreleased]: https://github.com/asterxsk/zet/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/asterxsk/zet/releases/tag/v0.1.0
