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

- **`zet-app`** — `cargo bench -p zet-app --bench latency`, because PRODUCT.md promised the latency
  numbers were "measured rather than assumed" and nothing anywhere measured one.
  - Three numbers, all of them zet's side of the line: cold start from `App::load` to the first
    character on the grid, a keystroke from the event to the bytes at the pty, and output into the
    grid in MiB/s over a megabyte of the mixed SGR and text a build log is made of. The payload is
    built rather than read, so the number does not move with the speed of the disk it came off.
  - Not keystroke-to-pixel, and the file says so: the last step is the present, which needs a device
    and a window. What is here is everything before it, which is what tells a latency you feel apart
    from the shell's.
  - It prints and does not fail. A timing threshold on a shared runner is a flaky test with a longer
    name, and criterion 1's 300 ms is written as a check on a real machine. CI runs it in its own
    job on every push so a regression is visible in the log without ever gating a pull request.
  - The first run on this machine: 253 ms to a PowerShell prompt, 0.4 µs per keystroke, 50 MiB/s.
    Criterion 1's budget has less room in it than it looks like from the outside, which is exactly
    the kind of thing that was invisible while nothing measured it.
- **`zet-render` / `zet-app` / `zet`** — hyperlinks, which were parsed and then neither drawn nor
  followed.
  - `zet-vt` has stored every OSC 8 target since the sequence was implemented, `Cell::link` has
    carried the index into the table, and `Term::link_for` is the only way to read one — with no
    caller anywhere in the workspace. PRODUCT.md's third promise lists OSC 8 hyperlinks among the
    rendering modes that work, and a link that draws as ordinary text and cannot be followed is the
    one thing a link is not.
  - A cell carrying a link is drawn with an underline whether or not the program asked for one,
    because the sequence says which URL a run of text points at and says nothing about how it
    looks. Storing the target and drawing the text plainly is drawing a link as ordinary text: the
    text is identical and the target is invisible.
  - Ctrl+click follows it, through `ShellExecuteW` with the `open` verb — not `cmd /c start`,
    because the URL was chosen by the program that printed it and a shell would parse it. The
    scheme is checked first, in `zet-app`, where the decision is testable: only `http`, `https` and
    `mailto` reach the host, because the system's open call runs a path rather than opening it, and
    a program that wants to run something should not be able to do it under a click on what looks
    like a link.
  - `crates/zet-app/tests/link.rs` drives a real `cmd.exe` into publishing one through its own
    prompt — the reason `scroll.rs` and `hold.rs` exist, in the same shape: the app, the parser,
    and the renderer each have to agree about the same cell, and nothing but a real program proves
    they do.
- **`zet-render` / `zet`** — `window.background`'s picture, the last of the three kinds the schema
  has documented since it was written, and the last inert key in the `[window]` section.
  - Decoded by GDI+, through the `windows-sys` declarations the crate already had, rather than by a
    PNG decoder added to the workspace: Windows has been the image path since XP, reads PNG, JPEG,
    BMP, GIF, and TIFF without being told which is which, and is already installed on every machine
    zet runs on. A dependency that parses a format designed for hostile input is a supply chain and
    an afternoon, and it would do a worse job.
  - `PictureQuad::new` decides the crop on the CPU and the shader only samples: scaled to fill, so a
    picture wider than the window loses the same amount off each end rather than being stretched or
    letterboxed, and a window dragged narrower keeps showing the middle of the picture. The
    aspect-ratio arithmetic is the part that is a one-line test here and an afternoon of squinting
    on the device, which is why it is not in the shader.
  - The bytes GDI+ returns for `PixelFormat32bppARGB` are B, G, R, A in memory, and the module
    converts them to the premultiplied RGBA a texture wants in one pass. Both halves of that are
    caught by a test against a four-pixel PNG committed under `crates/zet/tests/data/`: leaving the
    swap out is a picture of a different colour that still looks like a picture, which is a bug no
    amount of looking at the code finds.
  - A picture with a side over 8192 is drawn into a smaller bitmap before it is read, with GDI+'s
    bicubic filter. That is the largest texture `wgpu` will make, and the first attempt at this
    panicked — `Dimension X value 9000 exceeds the limit of 8192` — on a 9000-pixel-wide source,
    which is an ordinary size for a photograph and a wallpaper. The resize is exact integer
    arithmetic, unit-tested for the aspect ratio it has to preserve.
  - Uploaded once, when the configuration names a different file, and only then: the opacity rides
    in the frame like every other colour, so turning a picture down does not re-read it, and the
    window's shape is not baked into the texture, so resizing re-crops rather than re-decodes.
  - The picture is drawn where the gradient is, before every batch and over the theme's ground, so
    the two kinds of backdrop are one slot in the frame rather than two paths through the device.
- **`zet-render` / `zet`** — `window.background`'s gradient, which the schema has documented since it
  was written and which nothing drew.
  - A third shader and a third pipeline, and a `Frame::backdrop` that is *not* a batch: it can only
    ever be the first thing drawn, so a run would give a caller a way to get the order wrong that
    buys nothing. It sits beside `clear` because it is the same kind of thing — what the frame is
    painted on — and a frame with a gradient differs from one without in a single value rather than
    in the shape of the list.
  - The arithmetic that turns an angle into an axis is on the CPU, in `Gradient::new`, and is
    unit-tested there against the formula the shader runs. A length that ignored the angle would run
    out of colour before the far corner and clamp for the rest of the rectangle, which reads as a
    hard edge in a soft background; the tests fail for exactly that change.
  - The grid stops painting its ground when there is a backdrop, and that is the whole of how a
    background shows through a terminal: a cell whose colour is the theme's paints nothing already,
    and the one rectangle that stood for all of them was the thing standing in the way. A cell a
    program gave a colour to still paints it.
- **`zet-app` / `zet-ui` / `zet`** — the settings panel, which is the config file with a face on it.
  - `Ctrl+Shift+Comma` opens a 380px panel over the right edge of the grid. The terminal stays
    visible behind it and keeps updating: the preview is not a preview.
  - Rows are Appearance (theme, text scale, reduce motion, forced colours, window opacity), Tabs
    (position), Terminal (font, size, cursor shape, blink, and thickness when the shape has one),
    and Keys (one row per action). Every row writes straight through to `config.toml` as it changes, and
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
- **`zet`** — `--version` names the build it is, not just the release it is part of.
  - `zet 0.1.0 (4eb42e4, x86_64-pc-windows-msvc)`. `zet-update` has had `version_line`, `GIT_SHA`,
    and `TARGET` for a while, along with tests for all three, and nothing called any of them: the
    binary printed `CARGO_PKG_VERSION` and stopped there. A version on its own does not identify a
    build — two binaries can both call themselves `0.1.0` and differ — and the build script, the
    release workflow, and the security policy all assumed the commit was in there.
- **`zet-render`** — a machine with no usable GPU adapter gets a terminal rather than a dialog.
  - `request_adapter` was asked once, and a machine in a VM with no graphics acceleration, or
    over remote desktop with GPU redirection off, enumerates no adapter at all — so the only
    thing between that machine and a working terminal was a message box saying zet could not
    start. It is asked a second time with `force_fallback_adapter`, which names the software
    rasteriser: WARP on Windows, and always present.
  - Asked second rather than preferred, so a real GPU is still the default. WARP is a correct
    terminal at a fraction of the speed, which is the right trade for a fallback.
  - Verified rather than assumed: the test asks for the fallback adapter by name, opens a
    device on it, and pushes a frame through the same pipelines. On a machine that has a GPU
    that is the only way to run the second request at all, and it is the request that would
    otherwise never execute anywhere.
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
- **`zet`** — `--check-update` and `--update`, which are the updater wired into the binary.
  - `--check-update` asks GitHub whether a newer version has been published, prints the answer, and
    exits. It downloads nothing and changes nothing on disk, so a check you did not act on costs
    one request and no more. A check that cannot be completed exits non-zero, so a script can tell
    "there is nothing newer" from "there is no answer"; whether there *is* a newer version does not
    change the exit code, because this reports and does not decide.
  - `--update` is the same check and then the install: download, digest check, replace the running
    binary. Nothing is written until the download has matched the digest the release published, so
    the worst case of a build that has gone wrong is a message and no file. Windows will not let a
    running image be written to or deleted but will let it be *renamed*, which is the whole
    mechanism — the new version takes effect the next time zet starts.
  - Neither reads the config file, and there is nothing to read: zet makes no request on its own,
    so these two flags are the only way to make one. That also means both work on a machine whose
    config will not parse, exactly when someone might want them.
- **`packaging/`** — a per-user Windows installer and the icon it installs.
  - Inno Setup, installing to `%LOCALAPPDATA%\Programs\zet` with no elevation. Optional PATH
    entry, Start Menu shortcut, and "Open zet here" context menu.
- **`zet-vt` / `zet-input`** — the kitty keyboard protocol, both halves of it.
  - `zet-vt` tracks the flag stack: `CSI = flags ; mode u` sets, ors, or and-nots the flags,
    `CSI > flags u` pushes and `CSI < n u` pops, and `CSI ? u` answers with the flags in force.
    The stack is bounded, so a program that pushes in a loop evicts its own oldest entry rather
    than growing the terminal's memory, and an empty pop resets every flag. The stack is per
    screen: a full-screen program that dies without popping leaves nothing behind for the shell
    underneath, and `ESC c` clears it while `DECSTR` — a soft reset — does not.
  - `zet-input` encodes against it as `CSI code ; modifiers:event ; text u`, where the code is
    the key *before* shift. The protocol is explicit about that, and it is the part a naive
    implementation gets wrong: a program matching `ctrl+shift+a` looks for the code of `a`, so a
    terminal that sends the code of `A` hands it a chord it will never match.
  - The functional keys keep their legacy sequences, because in the protocol's own table those
    *are* the sequences. Keys the legacy table has no bytes for at all — the locks,
    `PrintScreen`, `Pause`, `Menu`, and `F13` up — are reported the protocol's way always. Text
    keys move to `CSI u` under `Report all keys`, or under disambiguation alone when the key is
    one the legacy encoding cannot tell apart. `Enter`, `Tab`, and `Backspace` are never
    disambiguated, which is the protocol's own carve-out: a shell has to be able to run `reset`.
  - Event types are only sent when the program asked for them. A release is otherwise not sent
    at all, because a program that did not ask would read one as a keypress.
  - `Report alternate keys` is deliberately not implemented, and the bit is dropped rather than
    echoed back. The host does not carry the physical key, and claiming a feature that is not
    there is worse than the honest no that the protocol's set-then-query handshake exists to get.

- **`zet-app` / `zet-ui` / `zet`** — a new tab asks which shell to open it with, which is what
  `tabs.open-default-without-asking` has been documented as switching off since the schema was
  written.
  - The key was read by nothing, so the answer was fixed: `Ctrl+Shift+T` always opened the default
    profile, and a machine with two shells and a preference had no way to say so per tab. With the
    key at its default of `false` the chord now opens a list of the discovered profiles, navigated
    with `Up` and `Down`, answered with `Enter`, and dismissed with `Escape` or by pressing the chord
    again. Holding the chord does not flap the list open and shut, because a modal question is not
    a step.
  - It is drawn as a fourth overlay beside the find bar and the two menus, in the same `surface-raised`
    with the same hairline and the same `signal` lamp on the lit row that the active tab carries, so
    the highlight in the list and the highlight on the strip are the same mark. Rows that do not fit
    are scrolled to rather than dropped, and a window too short to hold a caption and one row draws
    no popover at all — a question with no answers in it is worse than the default.
  - The chrome's hit regions now run back up the paint order rather than down it. They were pushed
    in the order things are drawn and `Chrome::hit` answers with the first region that holds the
    point, which happened to work for everything on the window until two overlays could be up at
    once: with the settings panel open behind the popover in a window narrow enough for the two to
    overlap, the panel's surface was pushed first and swallowed every click on a row drawn over it.
  - `zet-app` owns the state in one place, so the whole feature is unit-tested with no window: seven
    tests for the highlight (wrapping, a one-shell machine, an empty list, and what a choice and a
    change of mind leave behind), and ten more driving it through `App::key` and the host's own key
    path.
- **`zet`** — the icon is in the executable, and so is a version block.
  - `packaging/zet.ico` was reaching the installer and nothing else: `zet.exe` had no
    resource section at all, so the taskbar button, Alt-Tab, Explorer, and the Start Menu
    shortcut all drew the generic Windows application icon. A resource is a section of the
    PE file and has to be there before the linker is finished, so the only way in is a
    build script — `crates/zet/build.rs`, which hands the file to `winresource`.
  - The same section carries a version block, which is what Properties → Details reads:
    product name, description, version, publisher, licence, and the repository URL. All of
    it comes from the crate's own `Cargo.toml`, so none of it can drift from what
    `zet --version` prints.

### Removed

- **`zet-config`** — `Loaded.existed`, the fifth public item nothing called. The loader set it in
  three places and the only readers in the workspace were two assertions in
  `crates/zet-config/tests/config_io.rs`; `App::load` destructures the config, the path, and the
  diagnostics and has never asked. There is no first-save state to drive it either — `save` writes
  the file whether or not it was there, which is what makes the field's own doc comment describe
  behavior nothing has.
  - The half of that doc worth keeping moved to `load`, where the fact belongs: a file that is not
    there is not a diagnostic, and reading a config is not a way to write one.
- **`zet-vt` / `zet-pty` / `zet-config` / `zet-session`** — four public items nothing called, each
  with a doc comment naming a caller that does not exist. Found by reading the workspace's public
  surface against its own call graph; none is used by any crate here or by any test.
  - `zet_vt::cell::BLANK`. Every caller writes `Cell::blank()`, and the function is `const`, so the
    constant saved nothing a compiler does not already fold.
  - `zet_pty::PLATFORM_SUPPORTED`, which claimed to exist "so that the rest of the workspace can
    gate on it rather than repeating `cfg(windows)` in a dozen places". It was referenced nowhere,
    and it could not have replaced the gates it was written for: those are `#[cfg(windows)]`
    attributes, and a `cfg` predicate cannot read a runtime `const bool`.
  - `zet_config::theme::slug_of`, "for the settings panel's readout". The panel shows a theme's
    *name* and cycles through slugs it already has; the reverse lookup had no caller.
  - `zet_session::Session::process_id`, "for logging and for a host that has to hand the child to
    something else". zet does neither, and the doc's own note — the job object reaches the
    grandchildren a pid knows nothing about — is why the pid was never the right handle anyway.
- **`zet-config`** — the `[update] check-on-launch` key, and the `[update]` section it lived in.
  - It was reserved when the schema was written, for a check zet would make on launch, and the
    check was never built: eight places in the docs had to say the key did nothing. Wiring it up
    would mean a terminal that opens a socket because it was launched, on by default, with nowhere
    to put the answer — DESIGN.md defines no notification surface, and the settings panel is the
    config file with a face rather than a status board. `--check-update` and `--update` were
    already the whole of the feature.
  - Nothing breaks: no configuration in the world contains the key, because `config.toml` is
    written by zet and zet has never published a release. A hand-written file that does contain it
    now fails to parse, which is `deny_unknown_fields` doing its job rather than a silent no-op.
  - The privacy claim comes out of it stronger. There is no setting to turn anything off because
    there was never anything to turn off: the only request zet can make is one you typed.

### Fixed

- **`zet-app`** — the Problems section described the file as it was at launch, not as it is.
  - The rows are the loader's own words about the file rather than settings in it, which is what
    makes them worth reading and what makes them expire. The panel writes the file on every click,
    and a complaint about a value the user has just changed through the rows below it is the panel
    reporting a problem it fixed itself — with no way to be rid of it short of a restarting zet.
    A `theme = "nope"` that the user repicked from the theme row stayed on screen for the rest of
    the session.
  - The write moved from the host into `App::save`, which writes the file and then reads it back:
    the same question, asked of the same authority that answered it at launch, so the list cannot
    drift from the file it describes. The configuration is not taken from the re-read — a file
    something else has edited since must not walk back a click made in this window.
  - What the re-read does not fix stays. An unknown action name survives every write, because the
    file is edited leaf by leaf and nothing removes a line the user put there, and there is a test
    that says so: a save is not a way to make the panel stop complaining.
- **`zet-vt`** — two edits that lost text a program had already written.
  - A narrowing reflow anchored the screen by pulling the viewport up until the cursor was inside
    it. That pull is the bug: the cursor reflows above the top of the new screen exactly when the
    content grew below it, so following the cursor left the screen's last row short of the content's
    end, and every row in between went — the scrollback had already been cut at the anchor and held
    nothing below it, so there was no copy left to restore from. The screen is anchored to the
    bottom of the rebuilt content and nothing moves it; the cursor is clamped into the screen a few
    lines down instead. A cursor drawn on the wrong row until the program writes again is something
    a terminal recovers from, and a dropped line of a program's output is not.
  - `Row::write_at` cleared the cell a wide character landed on before checking whether the
    character fit there. In the last column it does not — terminals drop it rather than splitting
    it, and the cursor stays where it was — so the write drew nothing and took the character already
    in that cell with it. The check comes first now, and a write that draws nothing leaves the row
    as it found it.
- **`zet-input`** — four keys whose bytes on the wire were not the bytes the key means.
  - Ctrl+Backspace sent `0x7f`, the DEL that a bare Backspace sends. Every terminal that has one
    sends `0x08`, the C0 byte the chord has produced since the PC keyboard put Backspace under that
    row, and both the protocol's C0 table and its legacy-control table say so. A program binding
    `<C-Backspace>` apart from Backspace was handed one byte for two keystrokes, and the one it
    could not see was the one with the modifier on it.
  - `control_byte` knew `@`, `[`, `\`, `]`, `^`, `_`, `?` and the letters, and nothing about the
    digits above them on the same keyboard. `Ctrl+2` through `Ctrl+8` take the control code of the
    symbol above the digit — which is where `Ctrl+2` being NUL comes from — and `Ctrl+/` is `Ctrl+_`
    beside it and `Ctrl+~` is `Ctrl+^`. `0`, `1` and `9` have nothing above them and are left to the
    fallback, which is what the protocol does with every key its table does not list.
  - Kitty's event-type reporting was applied to keys that have no event to report. A key whose press
    goes out as text has no events at all, and Enter, Tab and Backspace have no release events
    unless every key is being reported as an escape code — the exception exists so that a user can
    still type `reset` at a prompt after a program that set the mode died without clearing it, and
    turning their release into a sequence is the one thing it is there to prevent. Their repeats
    still go as bytes, because a repeat is a press to anything reading them.
  - The X10 mouse form wrote its button field as a wide character where the form spends one byte per
    field. A parser reading it takes the next three bytes whatever they are, so a thumb button —
    the one code that does not fit in seven bits — arrived as two bytes of UTF-8 and put the reader
    two bytes ahead: the button read as `0xc2`, the column read as the row, and every report after
    it was read wrong.
- **`zet-app`** — the find bar's matches survived the grid under them being replaced.
  - The marks are painted by row and the count is a count of positions, both built from the
    terminal that was active when the search ran. Three things replace that terminal without going
    through `activate`, which is the one place that knew to throw the list away: closing the active
    tab, reaping a shell that exited, and resizing. In all three the bar went on showing a highlight
    and a `1 of 1` over a terminal that had never contained the query.
  - `pump` already touches the bar whenever a drain reports damage, which covers the common case of
    a reaped tab by accident — a shell that echoes the `exit` it was given has printed, and printing
    is what `pump` watches for. A shell that exits saying nothing is not covered, which is why the
    fix is at `reap` rather than left to the drain.
  - Closing or reaping a tab the user is *not* looking at does not touch the bar, because the
    terminal under the query did not move and searching the whole history again for that is work
    with nothing behind it. Both rules have a test, so the pair stays a decision rather than
    drifting into whichever one was written first.
- **docs** — a promise wider than the schema, and two comments describing work nobody did.
  - PRODUCT.md listed "font color, background color" among the things a user can change and there
    is no key for either, in the schema, in the key table, or in the settings panel. Both follow
    the selected theme, which is the design system's own rule — the grid gets exactly what the
    theme says — so the page now says that instead of promising a picker that does not exist. The
    background it *does* let you change is named properly: flat, a picture, or a gradient.
  - `zet-vt`'s `damage` module doc now says that its row-level readers are kept deliberately for a
    retained-buffer renderer and are called by nothing in production, rather than leaving a reader
    to work out which of five accessors the app actually uses. One is `is_empty`.
  - `zet_session::Session::visible_rows` promised that "a renderer is handed rows to draw and never
    has to know where the history ends". No renderer reads it: the renderer does the arithmetic
    inline, because it needs each row's position as well as the row and does not depend on the
    session crate at all. The comment now says what the function is for — the session's own tests.
- **`zet-app`** — the theme picker never said which palettes ship as their authors published them.
  - PRODUCT.md's seventh criterion promises that imported palettes "ship unmodified so they look
    like themselves, and the theme picker says so", and DESIGN.md repeats it along with the reason:
    a picker that showed them beside zet's own without a word would imply they had been checked for
    contrast. `Theme::published` has been set correctly on all eight themes since they were
    written, and both `theme.rs` and `imported.rs` say the settings panel reads it to know — the
    panel did not, so the criterion was kept by a field no user could see.
  - The Theme row is labelled `Theme (as published)` when the selected palette is one of the five
    imported ones. The marker is on the label rather than on the value, which is where the name
    goes: that column is 118px — wide enough for a chord and no wider — and the painter draws a run
    of text into the frame without clipping it, so `Solarized Light (as published)` would not be
    cut off at the control, it would run out of the panel and over the grid.
- **docs** — ten claims the code had stopped keeping, found by reading every page against the binary.
  - Security said the update check "is not wired in the current build at all, so there is no network
    path to attack yet". It is: `Args::CheckUpdate` and `Args::Update` are dispatched by
    `crates/zet/src/main.rs` and construct a `zet_update::Checker`, and the page now says what they
    do and that nothing checks on launch. The architecture page's `zet-update` section said the same
    false thing in two places, and its crate table a third.
  - Keybindings promised that "a chord is compared exactly, including the case of a character" and
    that `Ctrl+Shift+T` and `Ctrl+Shift+t` "are two different chords, and only one of them is
    bound". Letters are matched without regard to case, deliberately, because Caps Lock decides
    which one the system reports — `crates/zet-input/src/chord.rs` has said so in a comment and in a
    test since it was written. The same page's punctuation paragraph promised that a named key
    "fires whether your layout produces `<` there or something else", which holds only where the
    layout keeps the PC-101 pairing the matcher resolves shifted characters through.
  - Design capped cursor thickness at "1, 2, or 3px" in `docs/design.html` and in `DESIGN.md`. The
    range has been 1 to 8 since the three literals became one constant, and both the configuration
    reference and the panel's stepper say so.
  - Privacy listed "shell profiles" among what `config.toml` holds, in `PRIVACY.md` and in
    `docs/privacy.html`. Profiles are discovered from the machine on every launch and no key in the
    schema names one.
  - Security said releases are built "from a signed tag", in `SECURITY.md` and in
    `docs/security.html`. Nothing signs or verifies a tag: `README.md` documents the release step as
    a plain `git tag`, and the workflow's only tag check verifies the tag exists on the remote. Both
    pages now name what does stand in for provenance, which the second of them already described
    correctly a section further down.
  - Architecture listed `clipboard` and `platform` as "the only files in the binary with an
    `unsafe` block". `picture` — the GDI+ decoder behind `window.background`'s images — is a third,
    and `crates/zet/src/main.rs`'s own header made the same stale claim about the same file.
  - README and the site index said the update check "does not yet run on launch", which reads as a
    plan. It was dropped as not planned, and the index says exactly that twenty lines below.
- **`zet-app`** — a `[keys]` value that is not a chord was dropped without a word.
  - `parse_bindings`'s own comment said "the config loader has already reported the ones it could
    see", and `docs/keybindings.html` promises that "a misspelled modifier is reported as an unknown
    key". Both were false, and the loader's comment is the reason: `zet-config` does not parse
    chords — it has no business knowing what one is — and says so where it decides what it checks.
    So `new-tab = "Ctrl+Banana"` was looked at by nothing at all: the binding did not exist, the key
    did nothing, and no surface anywhere said why.
  - The app reports them now, through the same diagnostics the loader's own problems travel in, so a
    misspelled binding reaches stderr at startup and the panel's Problems section with everything
    else. `parse_bindings`'s doc no longer claims the loader did it.
  - Two values are deliberately not chords and are not reported: an action zet does not know, which
    is the loader's to report and already is, and the empty value, which is how the panel writes an
    action it has taken a chord away from.
- **`zet-app` / `zet-ui` / `zet`** — configuration diagnostics were produced and read by nobody, so a
  file zet had already repaired was a file with no way of saying so.
  - `Diagnostic` and `Severity` have been built by the loader since the schema was written, both
    references promise that "values that parse but cannot be used are reported separately", and no
    surface ever mentioned one: `App` held the list, and no window, panel, or stream drew it. A
    `cursor.thickness = 99` was clamped to 8 in silence, which leaves a typo indistinguishable from
    a setting zet ignored.
  - Every diagnostic is now written to stderr before the window opens, and listed at the top of the
    settings panel under **Problems**. Both, because neither is enough alone: a launch from a
    shortcut has no console anyone will read, and a launch from a shell has no window yet.
  - A problem is its own kind of settings row — `Row::Note`, alongside the headings and the
    controls — rather than a heading or a setting whose control went missing. It is a line to read,
    with the severity in the column a control's value would take, and it publishes no hit region: a
    row that answers a click by doing nothing is worse than one that does not answer.
- **`zet-app`** — `Ctrl+Shift+Home` scrolled to the bottom of the history instead of the top.
  - `ScrollToTop` asked the session to scroll by `i32::MIN` and `ScrollToBottom` by `i32::MAX`, and
    the session's sign convention is the other way round: a positive delta moves up into the
    history and a negative one moves towards the live screen. So the two keys were one keystroke
    with two names, and the oldest line in the buffer was unreachable from the keyboard — with
    `Ctrl+Shift+End` already showing it, the pair looked like it worked.
  - `crates/zet-app/tests/scroll.rs` is the test that found it and the reason it cannot come back:
    it opens a real `cmd.exe`, prints two hundred numbered lines, and asserts that the top of the
    history is on screen after one chord and gone after the other. Reverting the one word fails it
    with `Ctrl+Shift+Home did not move the view`.
- **`zet-config`** — a `[keys]` table replaced the whole default keymap instead of patching it.
  - `keys` was a plain map under `#[serde(default)]`, and serde applies a container default only
    when the field is *absent*. A file that had a `[keys]` table — which is to say, the file of
    anybody who has ever rebound a key — therefore loaded with exactly the bindings it named and
    none of the others, so writing one binding unbound the other seventeen and the panel showed
    seventeen rows reading `Unbound`. Both references promise the opposite, and both say it twice:
    "Rebinding an action replaces its default; you do not have to repeat the bindings you are
    keeping", and "a key you omit keeps the default above rather than becoming unbound". The
    file's entries are now laid over the shipped ones and win, which is also what makes a table
    naming all eighteen actions still mean exactly those eighteen.
  - Unbinding needs its own spelling, because it is the one thing omission cannot say any more:
    the settings panel takes a chord away from another action when you capture one it already
    holds, and a line that was deleted for that reason now reads as an action keeping its default.
    An empty value is that spelling — `close-tab = ""` — the panel writes it, and it is documented
    under both Keybindings and Configuration.
- **`zet`** — synchronized output was parsed and never honored, so every repaint flickered.
  - `DECSET 2026` is how a full-screen program says "what follows is half a picture, hold the last
    complete frame until I say otherwise". `zet-vt` set the flag and documented it as reported to
    the host, and no host read it: `App::pump`'s answer was discarded and every byte of output
    requested a redraw, so a program that cleared the screen and filled it in forty writes showed
    the user thirty-nine states nobody asked to see — and a TUI repainting on a timer showed them
    sixty times a second.
  - Output-driven redraws now wait for the program to finish, and only output-driven ones: a
    keystroke, a resize or the cursor's blink is the user's own business and is answered at once.
    The hold expires after 66 ms, because the marker has no end that a program which has crashed
    will send and a terminal that waited for one would be a window frozen on a stale frame for ever
    with nothing to say why.
  - The host is handed the instant the hold lapses rather than a yes or a no, and wakes for it:
    the one program that will never send the write that ends the repaint is the one the budget is
    for, and a held frame has nobody left to ask for it. `crates/zet-app/tests/hold.rs` drives a
    real `cmd.exe` into setting the marker through its own prompt and asserts the deadline to the
    millisecond, which is also what tells a budget measured from the start of the repaint apart
    from one restarted on every read.
- **`zet-render`** — every window with a gradient background stopped opening, and the unit tests for
  the picture were passing while it did.
  - The picture and the gradient draw in the same slot of the frame, and slot 0 has to hold a bind
    group for whichever pipeline is set. Binding the atlas's group in that slot moved from the top
    of `encode` into the picture's own branch, which left the gradient drawing with an empty slot —
    a validation error the device raises at the draw rather than a picture that comes out wrong. The
    failure was `The current set RenderPipeline with 'zet backdrop' label expects a BindGroup to be
    set at index 0`, and it took a live run with a gradient config to find, because the arithmetic
    the test suite covers was correct and only the draw was missing.
  - The slot is now bound before the branch and again after it, and the branch that changes it puts
    the atlas's group back. `crates/zet-render/src/gpu.rs` gained four offscreen tests over both
    backdrop kinds for it: every column of a gradient against the same interpolation the shader
    runs, every row of it to catch an axis with a flipped sign, the four quarters of an uploaded
    texture, half opacity over black, and a picture backdrop with no picture in it. Reverting the
    one line fails the first of them with the exact error above.
- **`zet`** — `window.opacity`, `window.start-maximized`, and `window.remember-position` were parsed,
  validated, documented, and read by nothing at all.
  - The window section was written with the schema, and the three keys that need the window itself
    never got the call that makes them real: the opacity reached no Win32 call, `start-maximized` was
    never passed to the window's attributes, and nothing wrote down where the window was. Setting any
    of them produced the default window, with no way to tell a typo from a feature that had not been
    built.
  - Opacity is a layered window — `WS_EX_LAYERED` and one constant alpha — set when the window is
    made and again whenever the section changes, so a window that was faded cannot differ from one
    that was born faded. `1.0` is not "255 by another name": the style comes off again, because a
    layered window is composited by the desktop rather than handed to the display controller, and
    paying that for an opaque window buys nothing. A terminal spends its life at the default, so
    the default is the one that costs nothing.
  - `start-maximized` is asked for when the window is created rather than called afterwards: a
    window maximized after it opens is a window the user watches jump. The remembered position is
    passed the same way and for the same reason.
  - The position is `%LOCALAPPDATA%\zet\window.txt`, holding one line of two numbers. Not a key in
    `config.toml`, because that file is the user's and a file that fails to parse produces defaults
    rather than an error — so a zet that wrote to it on the way out would be a zet that replaced a
    file it had just failed to understand. Per-machine rather than roaming, because a position is a
    fact about this desk. Read and written only when the corner is on a display that is attached
    now, which is what keeps an unplugged monitor's coordinates, and a window closed while
    minimized, from putting the next window somewhere nobody can see it.
- **`zet-vt`** — erasing after setting a background leaves the background behind, which is what
  `BCE` means and what zet was not doing.
  - The pen carried a background colour and the grid carried a second copy of its own, and nothing
    kept the two together. `ED`, `EL`, `ECH`, and the blank rows a scroll inserts all fill with the
    grid's copy, so `\x1b[41m\x1b[K` painted the theme's background over the rest of the line
    instead of red, and the same held for `\x1b[44m\x1b[2J` and for every scroll under a colour.
    They are one value now: every path that moves the pen re-syncs the grid's, and `apply_sgr`
    ends by re-syncing in case the parameter loop left the two apart.
- **`zet-input`** — a binding written in the config file matched nothing on a keyboard that
  reported the same key differently.
  - A chord is text: `Comma`, `BracketLeft`, `Plus`, or the character `[`. The host reports its
    own variant for the same physical key, Shift moves the character that sits above it, and Caps
    Lock inverts the case of the letter `winit` reports — so `Ctrl+Shift+T` was unreachable with
    Caps Lock lit, `Ctrl+Plus` was dead on every keyboard without a numpad, and the documented
    equivalence of `Ctrl+[` and `Ctrl+BracketLeft` was not true. Matching now compares the key and
    the modifiers separately, treats a letter as case-insensitive, and knows which keys carry
    Shift as part of their own name so that a bound `{` is not defeated by the Shift that produces
    one.
- **`zet-vt`** — a reflow could cut a double-width character in half.
  - Narrowing the window re-joined the logical line and re-split it every *n* cells, and a wide
    character landing on the last of those cells put its leading half at the right edge with its
    spacer opening the next row — a half glyph clipped by the window and a stray blank beneath it,
    which stayed on screen until something overwrote it. The pair now moves to the next row whole.
- **`zet-vt`** — erasing a row left its soft-wrap flag set.
  - A cleared row still claimed to continue into the row below, so a later reflow or copy treated
    it as part of a logical line: a screen cleared with `\x1b[2J` came back from a resize with a
    blank row's worth of spaces spliced into the middle of a line. An erase that covers a row from
    end to end now clears the flag; an erase of part of a row leaves it, because the tail is still
    the first half of what runs onto the next row.
- **`zet-vt`** — inserting or deleting lines left a stale soft-wrap flag above the edit.
  - `IL` and `DL` move rows sideways past each other, so whatever the row above the edit was
    continuing into is not what follows it any more — and the row pushed to the bottom of the
    region had its continuation pushed out with it. Both flags survived, which made a copy join two
    lines the user can see are separate and a search match across them.
- **`zet-vt`** — `ED 3` blanked the screen as well as dropping the scrollback.
  - The mode ran the whole `ED 2` row loop first. It is the history eraser, and the programs that
    send it — shell integrations, `tmux clear-history` — want their history gone without changing
    what the user is looking at.
- **`zet-vt`** — a second `CSI ?1049h` overwrote the saved cursor with the alternate screen's.
  - `enter_alt_screen` returns early when the screen is already alternate, but the save above it
    did not, so the nesting a full-screen program creating another one produces ended with the
    matching `1049l` restoring the user to wherever the inner program had left its own cursor.
    Both halves are now guarded on the switch actually happening.
- **`zet-vt`** — `CSI ?1005l`, `?1006l`, and `?1015l` turned the mouse encoding *on*.
  - The three name three ways of writing the same coordinates and the mode setter ignored the
    direction, so a program resetting `1006` on its way out left SGR encoding in force and zet went
    on sending `CSI < b;x;y M` to something expecting the legacy bytes. They now follow the same
    rule the reporting modes do: the last one set wins, and a reset only applies to the one it
    names.
- **`zet-app` / `zet`** — holding a chord bound to a toggle flipped the toggle on every repeat.
  - `action_for` rejected only releases, so a held `Ctrl+Shift+F` opened the find bar and shut it
    again at the auto-repeat rate, leaving it in whichever state the key happened to come up in —
    and the same for the settings panel and the tab strip's position. An action now says whether
    holding it is a request to repeat: a step (another tab, a larger font, another screen of
    scrollback) repeats, and a toggle runs once per press.
- **`zet`** — the settings panel swallowed `Tab` past its last row, so the panel never handed the
  keyboard back.
  - `Tab` off the end set the focus to `None`, and `None` was also what the panel starts in — so
    the next `Tab` re-entered at the top row and was swallowed too, and the shell could not be
    given a `Tab` at all while the panel was open. The two states are now told apart: a panel that
    has been tabbed out of takes nothing the panel names, and a click or the chord is what takes
    the keyboard back.
- **`zet-ui`** — the tabular-figures rule in DESIGN.md is met, and the note saying it was not is
  wrong.
  - `zet-ui` recorded that the chrome's numbers were drawn in Plex Sans's default *proportional*
    figures, because `zet_font::GlyphSpec` has no way to ask for an OpenType feature. The premise
    was unexamined: Plex Sans's figures are tabular by default, every digit at both weights advances
    600/1000 of an em, and the font carries no `pnum` feature to switch away from them. There was
    never anything to ask for, and a tab index, a font size, and a scroll position already line up.
  - That is a property of a file rather than of the code, so it is now held down by a test rather
    than by a paragraph: `zet-ui`'s `fonts` module rasterises all ten digits at both weights from
    the shipped `.ttf` and fails if they ever stop agreeing, which is what swapping the face for one
    with proportional figures would do.
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
- **`zet-input` / `zet`** — `Ctrl+Shift+Comma`, the shipped binding for the settings panel, could
  never fire.
  - A binding names a *key* and the event carries the character the layout put there, which under
    shift is `<` — and nothing compared the two. A chord now matches the character shift puts above
    the key it names, and only that character, and only for keys that are not letters, because a
    letter's case is the binding rather than a layout's business.
- **`zet-app`** — `Enter` in the find bar went to the previous match and `Shift+Enter` to the next.
  - The condition was inverted, and the two keys are one line apart, so it read as working until
    you were deep enough in the list to notice.
- **`zet-app`** — the find bar could not hold a space.
  - The space bar is a named key on this platform rather than a character one, so the bar saw no
    text and returned false — and the key fell through to the shell underneath, which is where the
    space went instead.
- **`zet-app`** — a find match with a row above the viewport was highlighted in the wrong columns.
  - Its start column was carried over from a row nobody can see, which, when the whole match mapped
    onto one visible row, handed the selection its corners the wrong way round and painted the gap
    between them. The same match taller than the window also scrolled its own first row off the top,
    so following one hid the half you can read.
- **`zet`** — a lost mouse release left the scrollbar grab set for the rest of the window's life.
  - Merely moving the mouse over the terminal then scrolled it with no button held. Losing focus and
    the pointer leaving the window now end every drag.
- **`zet`** — the settings panel closed twice for one `Escape`, and its chord toggled twice for one
  press.
  - The panel's bare-`Escape` branch had no event-kind guard, so the release was handled as well as
    the press. The same missing guard made the host schedule two redraws for every bound keypress
    instead of one.
- **`zet`** — the session is told its size at the end of a frame rather than from the window's
  resize event.
  - A resize event says how many pixels there are; how many columns that is depends on where the
    chrome puts the grid, and only the layout knows that. Fitting from the event fitted against
    the previous frame's layout and never corrected it, so a window opened at 1100x720 spent its
    whole life as an eighty-column terminal.
- **`zet-config`** — a binding the configuration no longer has stayed in the file.
  - The save merges the new settings into the file so that comments survive, and it only ever
    copied forward. Capturing a chord in the settings panel takes the key away from the action
    that held it, so the map the panel saves is missing an entry the file still had — and the file
    then said two actions held the same chord. On the next load both were bound, the first in sort
    order won, and the action the user displaced was back, holding the key they meant to give away.
    A save now drops what the configuration no longer has, which is also the only shape `load`
    accepts.
- **`zet-config`** — a comment inside an array, or on an inline table, was lost by a save that did
  not change it.
  - Whether a value had changed was decided by comparing its *rendering*, and an array with a
    comment between two of its elements renders differently from the same array without one — so
    an untouched setting was replaced and the annotation went with it. The comparison is the
    value's own now, ignoring how it is written, and a setting the file spells in one shape and
    the writer in another keeps the comment across the change of shape.
- **`zet-render`** — every glyph already in the atlas sampled the wrong texel rows once the atlas
  grew.
  - A placement stored its texture coordinates as a fraction of the atlas's height at the moment
    it was inserted, and growing the texture downward doubles that height. Nothing rebased them,
    and nothing could have: the quads for the frame being built are made before the growth and the
    texture is uploaded after it. A session that filled the first shelf and then grew drew its
    whole screen with the wrong rows — glyphs smeared into their neighbours' bitmaps. The
    coordinates are texels now, which do not move, and the shader divides by the size the texture
    actually has.
- **`zet-ui`** — a settings row scrolled half off the list drew over the tab strip.
  - The panel culled a row only when it was entirely off the panel, and there is no scissor under
    the painter: the rectangles and glyphs go straight into the frame. A row straddling the top
    edge put its control, its border and its value on top of the strip. A row is drawn once all of
    it is on the panel, which the panel's own padding is wide enough to absorb.
- **`zet-ui`** — the scrollbar could not be dragged while the settings panel was open.
  - The panel covers the window's right edge and the scrollbar is eight pixels of that edge drawn
    *over* it, so the thumb is visible — but its hit region was pushed after the panel's, and the
    first region holding a point is the one that answers. A click on the thumb landed on the
    surface beneath it and started nothing. The regions are pushed in the order the things are
    drawn now.
- **`zet`** — a click on the last pixel of the grid was reported to the program as one cell past
  its last column.
  - The grid's rectangle belongs to the window and is not a whole number of cells across, so a
    point in the leftover strip floored to one past the end and a wheel notch at the right edge
    sent column `cols + 1` to a program that has no such column. The cell is clamped to the
    cells the rectangle actually holds.
- **`zet-app`** — the release of a bound chord was sent to the shell.
  - The press runs the action and is swallowed, but the release took a different path and reached
    `encode_key` — which, for a program that has turned on the kitty keyboard protocol's
    event-reporting flag, encodes it. The program was told about the release of a key it never saw
    go down. A chord zet has claimed stays zet's on every event, which is now one question asked
    in one place.
- **`zet`** — the system's accessibility settings did not reach the app until something changed
  them again.
  - `Host::new` read them and kept them, and the only path that told the app returns early when the
    settings have not moved — which, for a session where the user does not reach into Windows'
    settings mid-run, is every frame. A user with "Animation effects" off got a blinking cursor
    for the whole session, and one with high contrast on got the ordinary theme. The app is told
    at construction now.
- **`zet`** — dragging out a selection with the cursor's blink turned off drew nothing.
  - A pointer move changed the selection and asked for no frame. Every other frame in the window
    is owed to something that answers — the program's echo, the blink timer — and on a still screen
    with the blink off there is nothing, so the sweep appeared only when the button came up. A
    drag now asks for its own frame, as the scrollbar drag next to it already did.
- **`zet-update`** — the binary a previous update displaced was never deleted.
  - An update cannot remove the copy it made, because it is running from that file; the cleanup
    that takes it away on the next launch existed and was documented as called at startup, and was
    called from nowhere. It runs on the path that starts the terminal now, and not on the one that
    performs an update — which is about to make the backup it would otherwise delete.
- **`zet-pty`** — a pseudoconsole leaked when a session could not be started.
  - `Pty::spawn` creates the console before the job object and before the reader thread, and either
    of those can fail and return straight out. Nothing closes the console on that path — no `Pty`
    exists yet — and it was dropped as a plain struct, so the pseudoconsole, both pipe ends and
    the conhost process behind them stayed for the life of zet. Once per attempt, so a user
    retrying a tab under the same pressure leaked a set each time. `Console` now closes itself
    when nothing else has, and the deliberate closes take the handle away first so none of them
    close it twice.
- **`zet-pty`** — a paste into a program that was not reading its input froze the window.
  - The console's input pipe is synchronous, so a write into it blocks until the pipe drains, and
    the caller is the thread that owns the window. Pasting a few megabytes into a program that had
    stopped reading stopped zet drawing frames and answering keys until the program read something
    or was killed. Keystrokes are one or two bytes and never showed it. Input is queued now and
    carried by a thread of the pty's own, so the most a caller pays is the copy.
- **`zet-config`** — the documented range for `cursor.thickness` was narrower than the one the
  settings panel offered.
  - The schema warned outside 1 to 3 and the panel's stepper went to 8, so a user who set a
    thickness the panel offered was told on the next launch that their own setting was out of
    range. The unit is *physical* pixels and is not scaled by the display, so a two-pixel bar is a
    hairline at 200% — the range is 1 to 8 now, in one constant rather than three literals.

- **`zet-config`** — a key inside `[window.background]` that the shape does not have was accepted
  without a word, and then deleted by the next save.
  - `Background` is the schema's one internally tagged enum, and serde does not carry
    `deny_unknown_fields` across one: `color = "#101010"` written beside `kind = "solid"` — which is
    what the gradient example leads you to write — parsed clean with no diagnostic at all. That
    silence is what made it a loss rather than a mistake. A save keeps only the shape a load
    accepts, so the key and the comment on it went on the next unrelated settings-panel edit, and a
    version-controlled config had a diff in it nobody asked for. The loader reports the key now, and
    says it will be dropped, which is the one thing that makes dropping it honest. Every other
    section refuses a key it does not have, so this is a check against the kind the file named
    rather than a second mechanism.
- **`zet`** — a click on the last column of the grid was reported one column to the left of it on a
  scaled display.
  - The host counts the columns the session has by multiplying the grid's logical width by the DPI
    scale and dividing by the physical cell; the pointer's clamp counted them by dividing by the
    cell already scaled. At 125% with an eleven-pixel cell those are 125 and 124.99999999999999, and
    the second one's floor is a column the program has and the pointer cannot reach: `vim` and
    `htop` got a click one to the left, a drag to the right edge stopped short, and a drag that
    ended there matched its own anchor and threw the selection away. The two are one function now,
    asked once, so the number the session was resized to is the number the pointer is clamped to.
- **`zet-ui`** — a section heading scrolled off the top of the settings panel left its rule behind.
  - The heading's text and the hairline under it were culled by separate tests against the same
    panel, and the hairline sits twenty-two pixels below the top of the text: for a band of scroll
    positions the text was gone and the rule was not, drawing a full-width line with nothing over
    it. They are one block, culled once.
- **`packaging/`** — uninstalling left zet's own entry in the user `PATH` if it had been written
  with a trailing backslash.
  - The installer accepts two spellings of the entry — with and without a trailing backslash — and
    declines to add a second copy of either. The uninstaller knew only the spelling the installer
    itself writes, so the entry it had declined to duplicate was the entry it then could not find:
    it reported "not in the user PATH" and removed nothing, leaving a `PATH` element pointing at a
    directory that was no longer there. Both directions ask one function where the entry is now,
    and how long the entry that was found is, rather than each working it out from the directory's
    own length.

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
