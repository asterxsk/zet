# Changelog

Notable changes to zet, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and zet adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The reasoning behind both, and what
counts as a breaking change in a terminal, is in
[Versioning](README.md#versioning).

## [Unreleased]

Until 1.0.0 ships, each release is a `0.x` minor and any of them may break compatibility; see
[Versioning](README.md#versioning).

### Fixed

- **`zet-render`** — a glyph's tint is scaled by its coverage, not by the whole texel.
  - The fragment shader multiplied the premultiplied tint by all four channels of the sampled
    texel instead of by its alpha. The texel's colour is white, so the tint's colour survived at
    full strength and every glyph drew as a solid rectangle of its own colour, in the right
    place and the right size, with only its edges dimmed.
- **`zet`** — the session is told its size at the end of a frame rather than from the window's
  resize event.
  - A resize event says how many pixels there are; how many columns that is depends on where the
    chrome puts the grid, and only the layout knows that. Fitting from the event fitted against
    the previous frame's layout and never corrected it, so a window opened at 1100x720 spent its
    whole life as an eighty-column terminal.

## [0.1.0] — unreleased

The first release, and the first point at which an archive is published. Not yet usable as a
daily driver.

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
- **`zet-config`** — the configuration file, its shipped themes, and the chrome palette.
  - TOML, with defaults, diagnostics that name the line and the offending span, and a round-trip
    save that preserves the user's comments.
  - A theme for the grid and a palette for the chrome, kept apart so neither can name the
    other's colours, and the 16-colour ANSI palette.
  - Appearance, cursor, font, tab, and window settings, and the keymap the input layer parses.
  - Hot reload: the file is authoritative and the settings panel is a view over it, so the panel
    edits the same `Config` the reload path reads back.
- **`zet-font`** — font discovery, cell metrics, and glyph rasterisation.
  - `fontique` for the system database and `swash` for the pixels, with the fallback policy
    supplied here rather than by a shaping layer.
  - A face chain resolved per character, with a resolution cache, and cell metrics taken from the
    primary face rather than from whichever fallback answered.
  - Every cell is the primary face's advance for `M`; a proportional or differently pitched
    fallback draws centred and is allowed to overflow the cell rather than being scaled into it.
- **`zet-input`** — decoded input turned into the bytes a program reads on its stdin.
  - The seam between the windowing layer and the pty, deliberately independent of `winit`, so a
    key is namable, parseable, and printable before it is ever an event.
  - Key and mouse encoding against the legacy VT / xterm table, plus the paste and focus
    sequences that a mode gates rather than a keystroke.
  - Chords and keybindings parsed out of the configuration, so `Ctrl+Shift+T` is a value before
    it is a binding.
  - The kitty keyboard protocol is not implemented — `zet-vt` does not track its flag stack — so
    `encode_key` reports the legacy encoding and answers `None` for key release and repeat.
- **`zet-session`** — one running terminal: a child process on a pseudoconsole, the parser
  chewing its output, and the grid that comes out.
  - A pump thread parked in `Pty::next_chunk` with a timeout, because a ConPTY output pipe never
    reports end of file; `Session::drain` then takes the bytes without blocking, which is what
    makes draining cheap enough to do every frame.
  - Scrollback over the grid, and a title, and nothing about windows, rendering, or
    configuration.
  - `Sessions` owns several and hands out the tab numbers; teardown stops and joins the pump
    before the pty is shut down, so a parked pump cannot keep a closed tab's child alive.
- **`zet-render`** — turning a terminal and an app into a frame.
  - The glyph atlas, shared with the chrome and told apart by the face in each glyph's key.
  - The grid and chrome pipelines, and the frame of quads both become, drawn from the theme's
    ANSI palette and the app palette respectively.
  - The device, the atlas, and the glyph cache; not the layout, which is `zet-ui`'s, and not the
    state, which is `zet-app`'s.
- **`zet-ui`** — the chrome: the titlebar, the tab strip, the caption buttons, the scrollbar, the
  settings panel, and the find bar.
  - Pure computation on the CPU: it fills a `zet_render::Frame` with rectangles and answers where
    the user just clicked, with no window and no GPU in the loop.
  - It holds a palette for the length of one call and has no way to name a theme at all.
  - The tab strip's indicator travel is measured against a time the caller supplies, so a
    transition can be asserted partway through.
  - IBM Plex Sans at two weights, self-hosted and registered into the font database, drawing
    from the same atlas as the grid.
- **`zet-app`** — what zet does when you press a key, with no window in sight.
  - The tabs, the selection, the scroll position, the keymap, and the cursor's blink, and the
    decision about what a key press means.
  - When it needs a window, a GPU, a clipboard, or an event loop it returns a `Command` and the
    caller carries it out.
  - Actions, the active theme, and the config reload path.
- **`zet`** — the binary: a window, a device, and an event loop.
  - A `winit` window and event loop, and the holder of the `HWND`, the surface, and the frame.
  - Key and mouse translation between the platform's events and the vocabulary `zet-app` speaks,
    and the waker the session's pump uses to wake the loop.
  - The Win32 clipboard, plain `CF_UNICODETEXT` in both directions, and the accessibility
    settings — high contrast and text scaling — read from the system rather than guessed.

### Security

- Update checks verify a `SHA256SUMS` digest before an archive is applied. Archives are not
  Authenticode-signed; see [SECURITY.md](SECURITY.md).

[Unreleased]: https://github.com/asterxsk/zet/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/asterxsk/zet/releases/tag/v0.1.0
