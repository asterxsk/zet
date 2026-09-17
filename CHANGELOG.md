# Changelog

Notable changes to zet, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and zet adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The reasoning behind both, and what
counts as a breaking change in a terminal, is in
[Versioning](README.md#versioning).

## [Unreleased]

Until 1.0.0 ships, each release is a `0.x` minor and any of them may break compatibility; see
[Versioning](README.md#versioning).

### Added

- **`zet-app` / `zet-ui` / `zet`** — the settings panel, which is the config file with a face on it.
  - `Ctrl+Shift+Comma` opens a 380px panel over the right edge of the grid. The terminal stays
    visible behind it and keeps updating: the preview is not a preview.
  - Rows are Appearance (theme, text scale, reduce motion, forced colours), Tabs (position),
    Terminal (font, size, cursor shape, blink, and thickness when the shape has one), and Keys
    (one row per action). Every row writes straight through to `config.toml` as it changes, and
    the file keeps its comments — `toml_edit` round-trips, so a note you wrote beside a setting
    survives being set from the panel.
  - Four controls: a choice steps through an enum, a stepper moves a number and stops at the
    ends rather than wrapping, a toggle flips, and a chord row asks for the next key you press
    and says `Press a key` until you press it. `Escape` cancels; `Escape` with nothing pending
    closes the panel.
  - The font row offers every monospaced family the machine has. Enumerating them is a registry
    walk and a face load per family, so it happens on its own thread during startup and arrives
    as a user event rather than being paid for on the frame that draws.
  - Binding a chord that something else already had leaves the other action `Unbound` and says
    so on its own row, rather than leaving the two of them to be resolved by whichever the map
    happened to yield first.
  - The panel is reachable from the keyboard: `Tab` or `Down` moves to the next row, `Shift+Tab`
    or `Up` to the previous, `Left`/`Right` (or `Enter`, `Space`) adjust the row the keyboard is
    on, and `Tab` past the last row hands the keyboard back to the shell rather than wrapping.
    Focus is drawn as a `hairline` in `ink`, not a `signal` ring — `signal` is the app's one lamp
    with a 3px by 40px budget, which a border around a 118-pixel control would spend several
    times over. A focused row below the fold is scrolled to, because a highlight nobody can see
    reads as a broken panel rather than a scrolled one.
  - Only those keys are the panel's. A letter, and any chord with a modifier on it, still goes
    to the shell: the terminal behind the panel is live, and typing into it is the reason the
    panel does not cover it.
- **`zet-vt` / `zet-app` / `zet-render` / `zet-ui` / `zet`** — the find bar, which was a picture of
  a find bar until now. `Action::Find` returned an empty command list and nothing ever called
  `Chrome::set_find_open`, so the binding drew a row and did nothing.
  - `Ctrl+Shift+F` opens a 32px row above the grid's bottom edge. The row's height comes off the
    grid rather than drawing over it, so opening the bar costs the terminal a row.
  - A match is a *logical* line, not a row: a row that wrapped is joined to the one below it, so a
    word broken across the fold is found and a match can straddle the two rows it covers. A row
    the program ended with a newline is its own line and stops there.
  - Case is insensitive unless the needle has a capital anywhere in it — `usb` finds `USB`, and
    `uSb` finds nothing. A wide character is matched as the character it is, and its match covers
    both of the columns it draws in.
  - The search is anchored where the last match was rather than where the view happens to be
    scrolled, so typing `beta` one character at a time narrows around the `b` it already found
    instead of chasing `b` down the scrollback.
  - `Enter` steps to the next match and `Shift+Enter` to the previous, wrapping at both ends, and
    a match that was off screen is scrolled the smallest distance that puts it on it. A match
    already visible does not move the viewport.
  - A match is the theme's `selection` colour laid over the cell at half alpha, and the one the
    arrows are on at all of it: a theme has one selection colour and no second highlight colour,
    which is what two weights of one colour are for. A cell a program gave a background keeps it.
  - While the bar is open it owns typing, because it is a text field, and only `Backspace`,
    `Enter`, `Shift+Enter`, `Escape`, and the toggling chord are its own.
  - The caret does not blink — the tab indicator's travel is the app's one authored moment. A
    query longer than the field shows its tail rather than its head, because the caret is where
    the next character goes. Past a thousand matches the count reads `1000+` rather than
    spending the frame counting.
- **`zet`** — a key that runs a bound action redraws the window.
  - The host only ever redrew when the pty echoed something, so `Find`, `FontLarger`,
    `ScrollPageUp`, and `ToggleTabPosition` all changed state invisibly. Typing still costs one
    frame rather than two, because the redraw is asked for only when the key ran a binding
    rather than being sent to the shell.
- **`zet-app`** — a key bound to a chord fires on the press and on every auto-repeat, and not on
  the release.
  - `bound` answers for a chord rather than for an event, so a release that reached it ran the
    action a second time: one press of `Ctrl+Shift+T` opened two tabs. Holding a bound chord down
    now repeats it, which is what holding it is asking for.
- **`zet-app`** — `[appearance]` is honoured, and each of its rows does what it says.
  - `text_scale` of `0.0` follows the system; anything else overrules it, and the chrome's type
    scales with the grid's. The field was stored and never read until now.
  - `follow_reduce_motion` and `follow_forced_colors` gate what the system reports. Reduce motion
    reaches the cursor blink for the first time: `App` had the flag and no way to set it.
- **`zet-ui`** — a tab shows its name after its number, and the window shows the active tab's.
  - `#3  PowerShell`: the number is the tab's identity and the name is what is running on it.
    The name is what the program set with `OSC 0`/`OSC 2`, or the profile's name when it set
    nothing, and it has been reaching `zet-ui` and being dropped on the floor until now.
  - A name too long for its cell is cut and given an ellipsis; a name that fits whole is never
    cut to make room for one. A cell is at most 180px wide, and past the point where the names
    fit every cell gives up the same amount rather than the first tab taking the row.
  - The OS window title becomes the active tab's name and then `zet`, so the taskbar and
    Alt-Tab can tell two zet windows apart.
- **`zet`** — `-d, --directory <path>` starts every tab of the window in that directory.
  - A property of the window rather than of its first tab: a window opened from "Open zet here"
    belongs to that folder, and a new tab in it that started somewhere else would be the
    surprising thing. The Windows installer registers a shell context menu that uses it.
- **`packaging/`** — a per-user Windows installer and the icon it installs.
  - Inno Setup, installing to `%LOCALAPPDATA%\Programs\zet` with no elevation. Optional PATH
    entry, Start Menu shortcut, and "Open zet here" context menu.

### Fixed

- **`zet-ui`** — the window's caption buttons are drawn as geometry, not as characters.
  - `−`, `□`, and `×` came from IBM Plex Sans, which the chrome loads with no fallback chain:
    a mark Plex does not carry draws `.notdef` — a box — in the place of a window control. The
    same characters were also the wrong shapes at the wrong weight. They are now a bar, a
    hollow square, two overlapping squares, and a cross, rasterised against the device pixel
    grid so a one-pixel stroke lands on one pixel.
- **`zet`** — holding a key down repeats it.
  - `encode_key` returned nothing for anything that was not a press, so a repeat was dropped
    and the keyboard stopped working the moment a key was held: holding an arrow key walked
    nowhere and holding Backspace deleted exactly one character.
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
