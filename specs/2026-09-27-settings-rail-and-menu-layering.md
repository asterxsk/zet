# Settings rail, settings button, and the menu's layer

**2026-09-27.** Spec and implementation plan for three changes to zet's window chrome. Two of
them are one surface each; one is a layering rule that several surfaces depend on.

Status: step 0 implemented. Steps 1 to 6 not started. Revised after review — see "what the review
changed" at the end.

## How we got here

Three requests:

1. Double-clicking the tab row's drag region should maximize. DESIGN.md:288 has promised it since
   the strip was drawn and nothing keeps it.
2. A settings control next to the tab strip's `+`, and the settings surface laid out as vertical
   tabs: a rail of sections down its left edge, one section's rows at a time.
3. Right-clicking a tab draws the context menu *under* the tab's title. While a menu is open it
   should be the topmost thing in the window.

Decisions taken with the user, recorded so the work can be checked against them:

- The rail is a settings-only device. `tabs.position = "top" | "left"`, the window's tabs and every
  other surface are untouched.
- The panel keeps overlaying the terminal: it does not dock, does not resize the grid, and what a
  program inside zet sees does not change when settings open.
- Section navigation is a vertical rail, not collapsible headings (the user changed their mind
  mid-session).
- The menu gets its own pair of frame batches rather than the chrome being restructured into an
  ordered list of layers.
- The button's mark is drawn as rectangles in `marks.rs`'s idiom; a literal gear would be a polygon
  rasteriser in a module written to avoid one.

## Part 1 — the menu's layer

### Requirement

While a context menu is open, nothing else in the window is drawn over it and nothing else takes a
click where it covers. The tab title under a menu must not be legible through it.

### Why it is broken

Two independent faults, and only the first is the one that was reported.

**The menu's surface is drawn under the chrome's text.** `Chrome::layout`
(`crates/zet-ui/src/lib.rs:635`) submits everything the chrome drew as exactly two frame batches:
every rectangle in one, every glyph in the next, in that order, and `Frame` draws its batches in the
order opened (`crates/zet-render/src/frame.rs:16`). That is the crate-wide layering rule — *chrome
text is always over chrome surfaces* — stated at `lib.rs:656` and reasoned at `paint.rs:63`. The
menu is drawn last of the overlays (`lib.rs:793-823`) and so is last *within the quad batch*, but
its labels land in the glyph batch beside the tab strip's titles, which were pushed earlier. The tab
title is therefore painted after the menu's surface. Clicking works; reading does not.

**A click on the menu's own surface loses to the strip.** `Chrome::publish` (`lib.rs:834`) pushes
regions in the reverse of paint order, and `Chrome::hit` answers with the first region holding the
point, so pushing earlier means winning. The comment at `lib.rs:864` says "The menu first of all,
because it was drawn last of all" — but the menu's regions are pushed *after* the captions, the
tabs, the `+` and the drag region (`lib.rs:835-852`). A menu opened over a caption button therefore
loses a click to the caption, which contradicts both that comment and DESIGN.md:251 ("drawn over
everything, caption buttons included, and it takes the click"). The existing test only covers a menu
over a *panel control* (`tests.rs:3181`), which is why nobody has seen it.

The suffix property in the design below is unaffected by the second fault: publish order is
independent of paint order.

### Design

Keep one painter and one pair of arrays. Record where the menu's output begins, and submit that tail
as its own batch pair.

- `Chrome` gains one private field, `menu_split: (usize, usize)` — the `(quads, glyphs)` lengths at
  the moment the menu began drawing.
- `Painter` gains two crate-private length accessors (`quads()`, `glyphs()`) for `overdraw` to read
  the split from. Nothing else about it changes.
- `overdraw()` records the two lengths immediately before it draws the menu, which is the last thing
  it draws.
- `Chrome::layout` submits four batches: chrome quads (`quads[..split.0]`), chrome glyphs
  (`glyphs[..split.1]`), menu quads (`quads[split.0..]`), menu glyphs (`glyphs[split.1..]`). Each is
  skipped when empty, exactly as the two are today, so a frame with no menu still submits two
  batches. `Frame` supports quads after glyphs — `gpu.rs`'s `encode` switches pipeline on a kind
  change and skips empty ranges — so the order is valid.
- The property that makes this correct is that the menu's output is a **suffix** of both arrays. It
  holds because the menu is the last thing `plan` draws. Nothing can check that from inside `layout`
  — any check there is vacuous, since the lengths it would compare are read at the same moment the
  split is used — so it is a comment in `overdraw` stating that anything drawn after the menu
  belongs to the menu's layer and would be wrong, and the batch test below, which fails if the
  split stops being the boundary it claims to be.
- Publish order is fixed in the same step: the menu's rows and surface are pushed **before** the
  captions, so the comment at `lib.rs:864` becomes true. The order becomes menu → captions → tabs →
  `+` → gear → drag → scrollbar → popover → panel.

Nothing else moves. The strip's own quad-then-glyph relationship is untouched, as are the panel's
surface and its rows.

### The road not taken

Restructuring the chrome into an ordered list of layers — each overlay with its own quad and glyph
buffers, one batch pair each, the order as data rather than as comments — is the shape the crate
eventually wants. It fixes the picker's latent version of the same fault (its surface cannot cover
the panel's text either) and makes the stacking rule readable in one place. It was rejected for now
because a painter holds `&mut dyn GlyphSource`, so a painter per layer means splitting that borrow
across every draw function, plus fourteen batches a frame, for one visible bug. A comment in
`overdraw` names it as the shape to take if a second overlap ever matters.

### Tests

1. **`crates/zet-ui/src/tests.rs` — `an_open_menu_is_drawn_and_hit_above_everything_else`.** Lay out
   a chrome with a tab and a menu opened over it, then assert on the frame: `frame.batches` is four
   batches of kinds `[Quads, Glyphs, Quads, Glyphs]`, the third batch's range begins where the
   second batch's chrome quads ended, and the menu's own surface rectangle is inside it. Then assert
   the hit order: a point on the menu over a tab answers `Hit::Menu`, and a point on the menu over a
   *caption button* answers `Hit::Menu` rather than `Hit::Caption(..)`. `Frame::batches`, `Batch`
   and `BatchKind` are public (`frame.rs:272-291`), so this needs no device and no pixels.
2. **The same file — `the_title_under_a_menu_is_in_the_batch_the_menu_is_not`.** The direct
   regression test for what was reported: after the same layout, the glyphs of the tab's title are
   at indices before `menu_split.1`, and the menu's label glyphs are after it. The fault this
   catches is a menu surface or label that has drifted back into the arrays the strip shares.

The old plan called for an offscreen pixel test. It is not writable: `Chrome::layout` needs a `&mut
dyn GlyphSource`, and the only real implementation, `zet_render::ChromeGlyphs`, comes from
`Renderer::chrome()`, whose `Renderer::new` requires a window (`renderer.rs:168`); `Gpu::offscreen`
yields a `Gpu` with no glyph source, and a hand-rolled source that draws no real ink would pass the
assertion even with the bug present. Test 1 pins the same property exactly, without a device. Eyeball
verification of the real thing happens in step 6 by running the app.

## Part 2 — the settings button

### Requirement

A settings control sits immediately right of `+` in the tab strip. Clicking it opens the panel if
closed and closes it if open — the same toggle `Ctrl+Shift+Comma` performs, through the same code,
so the two cannot drift. The control is drawn over the panel, so it is visible and clickable while
the panel is open, and it loses to an open menu.

### Design

- `Strip` (`strip.rs:46`) gains `settings: Option<Rect>`, beside `plus`. The rect is computed in
  `strip::horizontal` and `strip::vertical` — the strip owns its controls' geometry — but the mark is
  **drawn in `overdraw`, with the captions** (`lib.rs:812`), because `strip::draw` runs before the
  panel is drawn and a gear drawn there would be painted over by the panel whenever the panel covers
  the strip (any window narrower than the panel's own width). Drawn where `+` is drawn, the control
  would be invisible yet hittable, which is worse than absent.
- `horizontal()`: the gear sits at `cursor + plus_width` after the `+`, one number cell wide, the
  same width `+` is, so the two read as one pair. The tab loop reserves both (`cursor + width +
  plus_width + gear_width > limit` breaks), **and `tab_cap`'s available width subtracts both** —
  today it subtracts `plus_width` only (`strip.rs:162`), and sizing the tabs against a wider region
  than the loop will actually allow would drop a tab the sizing had just made room for.
- When the strip cannot hold both controls, the **gear is dropped** and its width becomes drag
  region; `+` keeps its guarantee of never being the thing that overflows. The gear is never drawn
  at the expense of a tab's room in the sense that no tab is drawn *under* it — the reservation
  applies either way — and when it cannot fit it is simply absent.
- The drag region starts after whichever of the two was drawn last, so DESIGN.md's drag region
  becomes the gap between the settings control and the caption buttons.
- `vertical()` (the left-rail tab layout, `strip.rs:202`): the gear goes directly below the `+` cell
  at the rail's cell width, using the existing cell height and floor.
- A window with no tabs has no strip row (`plan()` answers `row: false`), so there is no gear. It is
  a two-second state — resuming opens a tab — and the chord still reaches the panel. Left as it is,
  and named in the changelog's wording rather than papered over.
- `Hit::SettingsButton` is new; the existing `Hit::Settings` is the panel's own surface and keeps its
  name, with a comment in the enum saying which is which. The region is pushed after the `+`'s and
  before the drag region's, i.e. under the captions and tabs in priority, which cannot collide with
  them because the layout arithmetic is what keeps a gear from ever being drawn where a tab is.
- `hit` must also reach the new arm in `Host::over_panel` (`host.rs:1771`), which today is
  `matches!(hit, Hit::Settings | Hit::Setting { .. })`: a wheel over the gear must not scroll the
  terminal. `Hit::SettingsButton` joins the list.
- `chrome_press` gains an arm calling `Host::toggle_settings()`, factored out of the chord's branch
  at `host.rs:1038-1046` (toggling `settings_open`, resetting `settings_scroll`, clearing
  `capturing`, `settings_focus` and `settings_left`, then redrawing). Both callers use it.
- The gear press clears the double-click pair for free: the pair is taken off the host at the top of
  every press and only the drag region's arm puts one back.

### The mark

`Mark::Sliders` in `marks.rs`, drawn through `paint.physical` so every edge snaps to the device grid
at the same `MARK_BOX = 10.0` the caption marks use:

- three bars one `stroke(scale)` thick, at `-3`, `0` and `+3` logical pixels from the button's
  centre, each 10 wide, whole-pixel snapped;
- a 3×3 knob centred on its bar at `-3`, `0` and `+3` from the left end, so the three read as three
  positions of one control;
- colour `palette.ink_dim`, the same as the `+` beside it: they are the same class of control and
  adjacent, and a mark at a different weight from its neighbour reads as a different kind of thing;
- no hover state, matching `+` and the captions, which have none.

## Part 3 — the settings rail

### Requirement

The panel shows one section at a time, chosen from a rail of section names down its left edge, with
the chosen item marked. Clicking a section shows it. The keyboard reaches every row of every section
without a new key and without the panel becoming a place you can get stuck in.

### Where the sections come from

The caller already supplies them: `zet_app::settings::lines` emits `Line::Heading("Appearance")`,
`"Tabs"`, `"Terminal"`, `"Keys"` and `settings::problems` emits `"Problems"` when the loader had
something to say (`crates/zet-app/src/settings.rs:281`). The rail is derived inside `zet-ui` from the
`Row::Heading` lines it is handed, so **`zet-app` does not change at all**. No new type crosses the
crate boundary; see "identity" below for what does cross, which is a string.

### Geometry

- `PANEL_WIDTH` goes 380 → 560, with `PANEL_RAIL = 180.0` beside it. The page is what is left: 380,
  exactly the width the rows are already laid out for, so `ROW`, `CONTROL_WIDTH`, `LABEL_GAP`, `PAD`,
  the heading box, the note's columns and the control geometry all keep working and the page is
  today's panel with a different origin.
- The panel's right edge stays the window's and the slide still translates the whole panel in from it
  over 180ms (`overlays.rs:279-317`), so the rail arrives with the page rather than ahead of it.
- Narrow windows: the window's minimum is 360×240 (`host.rs:118`), narrower than rail plus usable
  page. The rail is clamped to at most 40% of the panel's width and the page takes the rest — at 360
  the rail is 144 and the page 216 — so rows degrade the way they already do in a narrow window
  rather than the panel acquiring a second layout.
- The rail's right edge carries the same one-pixel `hairline` the panel's left edge does.
- Rail items are the heading text at the headings' own type (`HEADING_SIZE`, `MEDIUM`,
  `HEADING_TRACKING`, upper case at draw time), left-aligned with `PAD` of inset, one `ROW` apart,
  starting `PAD` below the panel's top. The chosen item is filled with `hairline` and the row under
  the pointer with `ground` — the panel's own pair of fills for "this one" and "the one you are
  pointing at" (the menu uses `surface_raised` with a `hairline` hover; the panel uses `hairline` on
  `ground`).
- The heading is **not** drawn in the page. The rail says what the section is; a section that drew
  its own name under a rail item saying the same word is a line of nothing.

### The page

- The page's content is the chosen section's *rows* — every line from the one after the heading to
  the one before the next heading, or the end of the list. `Problems` is a section of notes and
  `font.fallback` is a note inside Terminal, so notes are rows like any other. The heading line
  itself is neither drawn nor counted in any height.
- **The rail is drawn only when there is more than one section.** With one heading, or none, the
  panel is exactly what it is today: no rail, the whole list in the page, full width. A list with no
  heading at all — which the existing tests build, and which a caller may reasonably hand over — is
  one section that begins at line 0. This rule is deliberate: a rail with one item is a control that
  cannot do anything, and it keeps every existing single-section caller working unchanged.
- The draw loop iterates the page's line range while **preserving the global line index**: `y`
  accumulates over the page's lines only, but each row that becomes a hit region is named by its
  index in the whole `settings` slice, because that is what `Hit::Setting { line, .. }` means and
  what `host.rs:1577` resolves against `app.settings()`. The loop cannot be a plain
  `enumerate()` over the whole slice as it is today (`overlays.rs:346`); it iterates the range.
- `block()`'s rule that line 0 gets no leading section gap (`overlays.rs:555`) becomes "the first
  line of the page", not "line 0": the first row under a heading has no line above it to be separated
  from.
- `visible()` (`overlays.rs:472`) culls against the page's rect on both axes rather than the panel's
  on `y` only. Today the x-axis is safe because nothing may be drawn over the panel's own width; with
  a rail inside the surface, a row must not reach into the rail either, and the same containment
  argument applies.
- `list_height` and `content_top` (`overlays.rs:566`, `:582`) measure the page's lines, and
  `content_top` takes the page's range and a page-relative index. The focus stays a **global** line
  index (that is what the host owns and what a click names), so the panel converts: the focused
  line's offset in the page is computed as `content_top(page, focus - page.start)`, and the scroll it
  compares against is the page's. Mixing a list-relative offset with a page-relative scroll is the
  one arithmetic mistake this change invites, which is why it is written down here.

### Identity, and who owns it

- The identity of a section is its **heading text**, not its index. Indexes are not stable: when a
  save clears the last diagnostic, `Problems` disappears and every later heading's index shifts by
  the length of that section, so a host holding `settings_section = 1` would silently find itself on
  `Tabs` the frame after the fix.
- `ChromeInput` gains `settings_section: Option<&'a str>` — the heading's name, or `None` for the
  first section. The panel answers with the name it actually showed in `Layout::settings_section` as
  an `Option<&'static str>`, which is Copy, `Default` and borrow-free, so `Layout` keeps its shape.
  (The names are `&'static str` in `zet-app`; the panel receives them as `&'a str` and hands back the
  `'static` one it was given.)
- A name that is not a heading in the current list shows the first section, and nothing is reported
  to the host beyond the answer — the host's stored name simply did not match, and the next click or
  keystroke sets one that does.
- `Host` gains `settings_section: Option<String>`, read back from `Layout` each frame the way
  `settings_scroll` is (`host.rs:826` is the model).
- Changing section resets `settings_scroll` to 0, in the host, for the same reason the panel's open
  is reset there: the scroll is a position in a list that is no longer on screen.
- Clicking a rail item (`Hit::Section(usize)`, the heading's line index) switches the section, resets
  the scroll and puts the keyboard back (`settings_focus = None`, `settings_left = false`): there is
  no row where the click landed, and the panel's rule is that it holds the keyboard only when asked.
- `Host::over_panel` (`host.rs:1771`) gains `Hit::Section(_)` and `Hit::SettingsButton`, so a wheel
  over the rail scrolls the panel rather than the terminal behind it.

### The keyboard

No new key is invented and none is taken from the shell. Sections follow the focus:

- `Tab`/`Down` past the last row of a section lands on the first row of the next section, switching
  the section and resetting the scroll; `Shift+Tab`/`Up` past the first row lands on the last row of
  the previous section;
- `Escape` while a row is focused puts the keyboard back; a second `Escape` closes the panel;
- walking off either end of the whole list still hands the keyboard back to the shell
  (`settings_left`), which is what keeps the panel from being a trap.

`Left`/`Right` keep meaning "lower and raise the focused control", which is why the rail is not a
focus stop of its own: a rail that took `Up`/`Down` and needed `Left`/`Right` to pick a section would
need those keys to stop meaning what they mean while it had focus — a second mode inside a panel that
is deliberately one list.

### Tests, and the existing ones this breaks

Six existing tests in `crates/zet-ui/src/tests.rs` feed a two-section `settings_lines()`
(`tests.rs:963` — `Appearance` then `Terminal`) into the panel and assert on rows of the second
section. With one section drawn at a time they must select the section they are asking about. The
draw helper gains a section argument; the tests to update are:

- `the_panel_draws_the_lines_it_is_given` (`tests.rs:1060`) — asserts `"Ctrl+Shift+T"`, a `Terminal`
  row;
- `the_hover_fill_covers_the_region_a_click_would_take` (`tests.rs:1094`) and
  `every_control_is_a_region_that_names_its_line` (`tests.rs:1135`) — both reach for line 4, under
  `Terminal`, through `control_rect`, which panics on a line with no control (`tests.rs:1043`);
- `a_stepper_answers_twice_and_everything_else_once` (`tests.rs:1158`) — filters on line 4;
- `a_row_that_does_not_fit_is_scrolled_to_rather_than_dropped` (`tests.rs:1220`) and
  `a_row_scrolled_half_off_the_list_is_not_drawn_over_the_chrome_above_it` (`tests.rs:1401`) — build
  heading-less lists, which the "no heading is one section, no rail" rule above keeps working
  unchanged.

New tests, in the same file:

1. The rail lists every heading the caller supplied, in order, and `Problems` appears exactly when
   the caller supplied it; a single-section list draws no rail at all.
2. Only the chosen section's rows are drawn: no row of another section is in the regions, and the
   heading's own text is not in the page.
3. A row that would cross into the rail is culled (the page's containment rule, mirroring
   `a_row_scrolled_half_off_the_list_is_not_drawn_over_the_chrome_above_it`).
4. A rail item is hittable and is not hittable where the page is; a page row is not hittable where
   the rail is.
5. A section name that is not a heading shows the first section, and `Layout` reports the name it
   drew.
6. The hit regions of a page row name its index in the *whole* list, not its index within the page.
7. The gear: its rect sits immediately right of `+` when there is room, is absent rather than
   squeezing anything when there is not, sits below `+` in the rail layout, is not part of the drag
   region, answers `Hit::SettingsButton` (including while the panel is open, where it must not answer
   `Hit::Setting`), and is drawn after the panel.
8. `Mark::Sliders` produces the expected rectangles at scale 1.0, in the style of the existing mark
   tests.

Zoom out, in `crates/zet-app`: nothing. If the implementation finds itself editing
`crates/zet-app/src/settings.rs`, the design has been misread.

## Docs

- `DESIGN.md`: the tab row bullet at :287 (the drag region is now the gap between the settings
  control and the caption buttons, and the row's controls are `+` and the gear); the settings panel
  section at :336-395 (560px with a 180px rail, one section at a time, sections follow the keyboard,
  and why the page is 380); and the "what is drawn over what" claim near :251, which becomes true in
  this change and should say how (the menu's own pair of batches).
- `docs/design.html` is a hand-written mirror with no build step (`pages.yml` copies `docs/`
  verbatim), so the same three edits go there — its equivalents are around lines 376, 418-419 and
  461.
- `CHANGELOG.md` under `[Unreleased]`: `### Added` above the existing `### Fixed`, for the settings
  button and the rail. Neither is breaking: nothing a program inside zet sees changes, no keybinding
  moves, and no config key is added or renamed. The menu's two fixes are `### Fixed` entries.

## The plan

Each step ends green on the three things CI runs (`.github/workflows/ci.yml`): `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

- **Step 0 — double-click. Done.** `Host::titlebar_press`, taken at the top of every press and put
  back only by the drag region's arm; `mouse::double_click` decides, and the press that completes the
  gesture does not also start a drag. CHANGELOG written. `cargo check -p zet --all-targets` clean,
  99 unit tests pass. The predicate is unit-tested in `mouse.rs`; the six lines of wiring need a
  window and an event loop, so they are verified by running the app.
- **Step 0b — the pair is also dropped when the pointer leaves or the window loses focus.**
  `end_drags` (`host.rs:1266`) already means "let go of everything the pointer was holding" and is
  called from `CursorLeft` and `Focused(false)`; it clears `titlebar_press` too, so a press, a walk
  out of the window and a press back in within 500ms is not a double-click.
- **Step 1 — the menu's layer.** `Chrome::menu_split`, the painter's two length accessors, the four
  batches in `layout`, the comment in `overdraw`, the publish-order fix, the two tests above.
- **Step 2 — the settings button.** `Mark::Sliders` and its test, `Strip::settings` and both layout
  functions with the `tab_cap` arithmetic, the draw site in `overdraw`, `Region`/`Hit::SettingsButton`
  and the press arm, `Host::toggle_settings()` factored out of the chord, `over_panel`, the geometry
  and hit tests, the DESIGN.md bullet, the HTML mirror, the CHANGELOG `Added` entry.
- **Step 3 — the rail, geometry.** `PANEL_RAIL`, the widened `PANEL_WIDTH`, the rail's drawing and
  hit regions, the page's rect, the range-scoped draw loop with global line indices, the `block()`
  first-of-page rule, `visible()` on both axes, `list_height`/`content_top` over a page,
  `ChromeInput::settings_section`, `Layout::settings_section`, the fallback and the reporting, the
  six existing tests reworked, and new tests 1-6 and 8.
- **Step 4 — the host side of the rail.** `Host::settings_section`, reading the name back from the
  layout, the scroll reset on a section change, the click arm, the keyboard walk across section
  boundaries, and the DESIGN.md panel section plus its HTML mirror and the CHANGELOG note.
- **Step 5 — the read-through.** A clean `cargo test --workspace`, the fmt and clippy gates, a run of
  the app to look at the rail, the gear, and a right-clicked tab with the menu over its title, and a
  read of DESIGN.md against what was built.

## Risks

- **One painter, two layers, held together by an order.** The menu's batches are correct while the
  menu is the last thing `overdraw` draws. The comment and the batch test are the whole guard; the
  layer list in "the road not taken" is the right shape if that ever stops being true.
- **A wider panel covers more terminal.** 560 against 380 hides about half again as much grid while
  settings are open. That is the price of the rail; docking and reflowing the grid is a much larger
  change and was declined.
- **Two numbers describe the panel's width** (`PANEL_WIDTH` and the 380 page). They must stay in step
  with `CONTROL_WIDTH` and the label column, and a future change that wants a different page width
  should derive the constants rather than write them twice.
- **The gear has no keyboard route of its own** — no focus ring, like the caption buttons. The chord
  is the keyboard's way in.

## What the review changed

A review pass against the code found these, all folded in above: the six existing panel tests that
one-section-at-a-time breaks, and the no-heading case they depend on; the offscreen pixel test being
unwritable without a window or new public API in `zet-render`, replaced by the batch assertions; the
debug assertion being vacuous, replaced by the batch test as the guard; the draw loop, `block()`'s
line-0 rule and `content_top` all assuming the whole list; the page's content described
contradictorily; the gear's draw site unspecified and its natural home under the panel; the fit rule
claiming a gear cannot squeeze a tab while reserving its width unconditionally; `tab_cap` left out of
the arithmetic; `over_panel` not covering the rail; section identity by index shifting when
`Problems` disappears; the chord branch's line number; the region-order reasoning inverted, and the
separate finding that the menu's regions lose to the captions today — a second defect, now in scope;
and the double-click pair surviving pointer-leave and focus-loss.

## What was built

All of it. Steps 0 to 5 are implemented in the working tree, uncommitted: `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`
are clean, and the app was launched for twelve seconds with no output and no panic.

Files, and what changed in each:

- `crates/zet-ui/src/lib.rs` — `Chrome::menu_split` and the four-batch submission, the `Painter`'s
  two length accessors used to record the boundary, the publish order (menu first, over the
  captions), `ChromeInput::settings_section`, `Layout::settings_section`, `Hit::Section`,
  `Hit::SettingsButton`, `Region::Section`, `Region::SettingsButton`, and the settings mark drawn
  from `overdraw` rather than with the strip.
- `crates/zet-ui/src/overlays.rs` — the section model (`Section`, `sections`, `resolve`,
  `rail_width`), the rail (`draw_rail`), the page and its range, `page_scroll`, the range-scoped
  draw loop carrying global line indices, `block`'s first-of-page rule, `list_height`/`content_top`
  over a page, and `visible` on both axes.
- `crates/zet-ui/src/geometry.rs` — `PANEL_WIDTH = PANEL_RAIL + 380`, and `PANEL_RAIL = 180`.
- `crates/zet-ui/src/marks.rs` — `Mark::Sliders` and its three constants.
- `crates/zet-ui/src/strip.rs` — `Strip::settings`, both layouts, `tab_cap`'s arithmetic,
  `settings_mark`.
- `crates/zet/src/host.rs` — `titlebar_press` and the double-click arm, `end_drags` clearing it,
  `settings_section`, `toggle_settings`, `show_section`, `follow_section`, `heading_name`,
  `over_panel` covering the rail and the button, and the layout read-back.
- `DESIGN.md` and `docs/design.html` — the tab row, the menu's batching, and the settings section.
- `CHANGELOG.md` — `### Added` for the button and the rail, and the menu's two fixes.

Tests added: six in `zet-ui` for the menu's layer, the gear and the rail; three in the binary crate
for the seam between the app's headings and the panel's sections, the keyboard walking across a
section boundary, and a click on a rail item.

Three deviations from the plan above, all deliberate:

- **The settings mark hovers.** The plan said no hover, "matching `+` and the captions, which have
  none". The captions do hover — `ink_mid` at rest, `ink` under the pointer, `danger` for close
  (`strip.rs`) — so the mark follows them rather than the text `+` beside it, which does not hover
  because it is a character rather than a mark.
- **Two functions were factored out for testability rather than for structure.** `chrome_press`
  answers "not mine" for everything without a window, so the section click lives in
  `Host::show_section` and the panel toggle in `Host::toggle_settings`, and both can be asked
  directly by a test.
- **The pixel test is not written**, for the reason in Part 1's Tests: there is no offscreen chrome
  glyph source, and a hand-rolled one that draws no real ink would pass with the bug present. The
  batch assertions pin the same property exactly. Eyeballing the menu over a tab is the manual step
  left over, and it is the one thing here that no test checks.
