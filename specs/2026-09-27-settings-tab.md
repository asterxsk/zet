# The settings tab: settings as a page of its own, and three corrections to the strip

**2026-09-27.** Spec and implementation plan for one change to the window: the settings UI stops
being an overlay beside the terminal and becomes a tab that owns the content area, the two bare
marks in the strip lose their glow, and the settings control moves next to the new-tab mark.

Status: not started. Revised after review — see "what the review changed" at the end.

## The request

"The hover animations for the plus and the settings button are completely off. And for the settings
button, move it closer to the plus button. Additionally, for the hover for both of these buttons,
instead of adding a background, just make the actual icon glow like the plus and the settings icon
will glow when hovered. Everything else is good, and also the settings window should open in full
screen in a different tab called settings, which can be closed. Make the sidebar elements on the
left not fully capitalized."

Answered before the plan was written, because each one changes what gets built:

1. **The tab replaces the panel.** The settings mark opens a settings tab. The overlay that slides
   in from the right edge, and its 180ms slide, are deleted.
2. **"Full screen" is the content area** — everything below the strip, beside the rail when the
   tabs are in a rail. The strip stays, so the settings tab is visible in it and is closed the way
   a tab is closed.
3. **No glow at all.** The `+` and the settings mark step up to `ink` over the existing 110ms
   hover fade and draw nothing behind themselves. Not a square, not a halo.
4. **The settings control gets a 16px cell**, tight against the new-tab mark: the two marks end up
   25.5px apart with 15.5px of clear space between them, down from 35.05px and 25px.
5. **Sentence case everywhere**, rail items and page headings alike.
6. **The settings tab has no number** — it reads `settings`, not `#3 settings`. The terminals keep
   their 1..=N numbering unbroken.
7. **It stays open when you leave it**: switching to a terminal leaves the settings tab in the
   strip, and the settings mark stays lit while it exists.
8. **Closed by the mark, by Escape, by the close-tab chord, and by a middle click** — the same
   gestures a terminal tab has, plus the mark.

## What is true today

### The strip

- A tab's identity is a `u32` and nothing else: `TabInfo { index: u32, title: String, hovered:
  bool }` (`crates/zet-ui/src/lib.rs:306`), where the number is "its place in the strip, from one,
  with no gaps". Every key the strip keeps about a tab is that number — `Region::Tab { index }`
  (`lib.rs:495`), `Hit::Tab(u32)` (`lib.rs:357`), the indicator's travel `from_index`
  (`strip.rs` `Travel`), and the cell itself (`strip.rs:50`, `index` at `:52`).
- A cell's width is its number's width plus padding, then its title: `number_cell` (`strip.rs:375`)
  and `tab_width` (`strip.rs:381`), where `tab_width` is `floor(number_cell) + TAB_GAP +
  advance(title)`. The number is always drawn: `tab_number` (`strip.rs:610`) emits `#` and the
  digits for every cell.
- The two controls after the run: `plus_width = number_cell(paint, 1)` and `settings_width =
  plus_width` (`strip.rs:177-178`), laid out end to end at `strip.rs:219-228`. Both are one number
  cell wide, so the two marks are 35.05px apart.
- The hover of the `+` and of the settings mark is `marks::glow` (`marks.rs:182`): three concentric
  *squares* of `ink` at 12%, 6% and 4%, clipped to the control's rectangle (`strip.rs:587`,
  `strip.rs:336`), plus a cross-fade of the mark's own ink.
- `number_cell` and the layout arithmetic live in `strip.rs`; `SETTINGS_CELL` does not exist.

### The settings panel

- The window owns the state: `settings_open: bool` and, beside it, `settings_scroll`,
  `settings_section`, `settings_focus`, `settings_left`, `capturing` (`host.rs:315`, through
  `settings_left` at `host.rs:353`).
- `overlays::panel` (`overlays.rs:454`) draws a `PANEL_WIDTH` (560, `geometry.rs:186`) surface
  against the right edge, translated in by `arrival` over `PANEL_SLIDE` (180ms, `geometry.rs:203`),
  with the rail (`rail_width` = `min(PANEL_RAIL, 0.4 * width)`) and the page beside it. It is
  drawn from `Chrome::overdraw` (`lib.rs:914`), which `plan` calls at `lib.rs:874`: strip, then
  panel, then find bar, picker, scrollbar, settings mark and menu. The settings mark is drawn
  *after the panel* (`lib.rs:935`) so that it stays visible over its own panel.
- `overdraw` is not a second pass the host runs: `plan` calls it, and the host draws the grid
  *before* the chrome at all — `host.rs:818` picks the session, `host.rs:830`
  `zet_render::draw_grid(..)`, `host.rs:846` `chrome.layout(..)`. The grid is already under every
  chrome quad, which is why the panel could be drawn anywhere in the chrome and still be on top of
  the terminal.
- It deliberately does not resize the grid: `let grid = Rect::between(left, top, size.width,
  size.height - bottom)` (`lib.rs:880`) is computed without reference to the panel, and
  `overlays.rs:442` says so. The terminal behind it stays visible and keeps running.
- Keys: `panel_key` (`host.rs:1194`) takes Tab, the four arrows, Enter, Space and Escape while
  `settings_open`, and every other key falls through to the shell — "a letter is still a letter for
  the shell with the panel up" (`host.rs:1114`). Wheel over the panel scrolls the panel
  (`host.rs:1851`, `over_panel` at `host.rs:1913`).
- Mouse: `forward_motion` (`host.rs:1412`), `forward_mouse` (`host.rs:1591`) and `wheel_to_program`
  (`host.rs:1856`) all aim at `self.app.active()` whether the panel is up or not, and `grid_cell`
  (`host.rs:1753`) resolves a point in the grid rect to a cell. With the panel up, a program that
  asked for mouse reports keeps getting them for the sliver of terminal still beside the panel.
- `Host::toggle_settings` (`host.rs:1325`) flips the bool and lets go of the scroll, the focused
  row, the capture and the walked-off-the-end flag.
- The rail and the page both upper-case their headings in the painter: `&section.name
  .unwrap_or_default().to_uppercase()` (`overlays.rs:412`) and `&setting.text.to_uppercase()`
  (`overlays.rs:548`). The app's own literals are already sentence case — `Line::Heading("Problems")`
  (`crates/zet-app/src/settings.rs:281`), `"Appearance"` (`:334`), `"Tabs"` (`:407`),
  `"Terminal"` (`:427`), `"Keys"` (`:478`) — so the upper case is the painter's doing and only the
  painter has to stop doing it.

## The design

### One: a tab that is not a shell

The strip's key for a tab becomes an enum, because a tab is no longer always a numbered shell:

```rust
/// Which tab a cell in the strip is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabId {
    /// A shell, by its session number: 1..=N, no gaps, no reuse.
    Terminal(u32),
    /// The settings page. At most one, always the last cell, and it has no number.
    Settings,
}
```

- `TabInfo.index: u32` becomes `TabInfo.id: TabId` (`lib.rs:306`).
- `ChromeInput.active: Option<u32>` becomes `Option<TabId>` (`lib.rs:100`).
- `Hit::Tab(u32)` becomes `Hit::Tab(TabId)` (`lib.rs:357`), and `Region::Tab { index, rect }`
  becomes `Region::Tab { id, rect }` (`lib.rs:495`): the settings cell answers
  `Hit::Tab(TabId::Settings)`, which is what a click on it activates.
- `strip::Travel`'s `from_index` and `strip::TabCell::index` become `TabId`. Nothing about the
  travel changes: two identities, one from and one to, the same as two numbers were.
- The cell's width for `TabId::Settings` is padding, the title, and padding — no `#`, no digits, no
  `TAB_GAP`. `number_cell` takes the `TabId` and answers `2.0 * TAB_PADDING` for a settings cell
  (its own padding, and no number), and `tab_width` adds `TAB_GAP` only for a terminal cell: a gap
  between a cell and its number that is not there is a gap of padding, which the cell already has.
  That second special case is the price of a cell with no number, and it is one `if` in the one
  function that already computes a cell's width.
- The settings tab's title is the word `settings`, passed by the caller like any other title. The
  chrome does not spell it: it draws what it is given, exactly as it does for a tab a program
  renamed.

The run is unchanged otherwise. Hover, the preview bar, the cross-fade and the indicator's travel
all work on the settings cell because they are all keyed by `TabId` now.

### Two: the page

`overlays::panel` becomes `overlays::page`, and it is given the rect it fills instead of working one
out from the window's right edge:

```rust
pub(crate) fn page(paint: &mut Painter<'_>, input: &ChromeInput<'_>, content: Rect, hover: &Hover) -> Page
```

- `content` is the grid rect the chrome already computes — `Rect::between(left, top, size.width,
  size.height - bottom)` from `plan` (`lib.rs:880`). The page and the grid occupy the same
  rectangle, which is the whole meaning of "a tab": what is on screen is one or the other, never
  both.
- The surface is still `surface_raised` behind its own left hairline where the rail is, for the
  reason the panel was: the contrast floors are stated against that colour, and a page that is the
  same colour as the terminal behind it is a page with no edge.
- The rail is `min(PANEL_RAIL, 0.4 * content.width)` — unchanged arithmetic, a different width.
- The slide is deleted: `arrival`, `panel_slide`, `panel_was_open`, `retarget_panel`,
  `panel_arrival`, `geometry::PANEL_SLIDE_MS`, `PANEL_SLIDE` and the whole of `Chrome`'s second
  transition go with it. A page that a tab switch has already swapped to is not a thing that slides
  in from an edge, and the mechanism that made the panel arrive is now a special case with no
  caller.
- `PANEL_WIDTH` goes too, and with it the "narrower than the panel means the panel is the window"
  clamp: a page is as wide as the window because that is what a tab is.
- The page keeps the panel's place in the draw order — the same call in `overdraw` (`lib.rs:914`),
  between the strip and the find bar — because the draw order was never what made it an overlay.
  The host draws the grid before the chrome (`host.rs:830` then `:846`), so everything the chrome
  draws is already over the terminal, and a page drawn here is over the terminal for the same
  reason the panel was. What settles that the page is not an overlay is the grid gate in §Seven,
  not the order of the quads.
- The settings mark stays where it is in the order too (`lib.rs:935`, after the page). It no
  longer covers anything — the page ends where the strip begins — but moving it buys nothing and
  would separate it from the comment that explains why it is drawn last.
- Regions: the page publishes `Region::Settings(content)` for its own surface first and its rail
  items and controls after, exactly as the panel did. The published surface is now the whole
  content rect, so `Hit::Settings` means "on the settings page and not on a control of it" and the
  wheel keeps working anywhere over it.
- `Panel { scroll, shown }` keeps its name and its two answers, and `Layout.settings_scroll` /
  `Layout.settings_section` are unchanged: the page is still the only thing that knows how tall its
  content is.

`ChromeInput.settings_open` is **deleted**. The chrome does not need to be told that settings are
open: `input.active == Some(TabId::Settings)` already says the page is the content area, and
`input.tabs` containing a `TabId::Settings` already says the tab exists. Two inputs where one
already answers is how a chrome and a caller start disagreeing.

### Three: the settings mark

The mark is `ink_mid` when there is no settings tab and `ink` when there is, with the hover
cross-fade on top of that — so it reads as lit for as long as the page exists, the way the active
tab's bar does. It stays drawn with the strip and no longer needs to be drawn after the page: the
page is *below* the strip now, and a control drawn over something it is not covering is a control
with a reason nobody can see.

### Four: the two bare marks lose their glow

`marks::glow`, `GLOW_RINGS`, `GLOW_STEP`, `GLOW_ALPHA`, `strip::PLUS_GLOW` and the glow calls in
`strip::draw` and `strip::settings_mark` are deleted. What is left is the cross-fade that was
already there: the `+` goes `ink_dim` to `ink`, the settings mark `ink_mid` to `ink`, both over
`HOVER` (110ms). Both marks are drawn twice for the length of the fade, at complementary
coverages, which is the same pair of draws the tab numbers have always used.

This deletes the only place in the chrome that drew a rectangle the user did not ask for. The three
rectangles that were a glow were a square behind a mark, and a square behind a mark is a
background — which is the thing the request says it is not.

### Five: the settings control

`geometry::SETTINGS_CELL: f32 = 16.0`, and the settings control is that wide instead of one number
cell. The pair is laid out as before — the settings cell begins where the plus cell ends — so the
two marks end up 25.5px apart instead of 35.05px, with 15.5px of clear space between two 10px
marks.

The drop rule is unchanged and now cheaper: a 16px control needs less room, so the window in which
both fit is 19px narrower than it was, and the settings control is still the one that goes.

DESIGN.md's "New tab: a `+` at the end of the run, the footprint of a tab with no name" is
untouched: the plus keeps its cell. The settings control is not a tab that is about to exist, so it
gets a cell the size of what it is.

### Six: sentence case

`overlays.rs:412` and `overlays.rs:548` drop `.to_uppercase()`. The app's literals are already
sentence case, so nothing on the app side changes and the rail reads `Appearance`, `Tabs`,
`Terminal`, `Keys`, `Problems`.

DESIGN.md's "12px uppercase with a hairline under it" becomes "12px with a hairline under it", and
the rail's own rule follows.

### Seven: the host's state machine

One field replaces the panel's bool and the tab's existence, because there are three states and two
bools would let one of them lie:

```rust
/// The settings tab: whether it exists, and whether it is the one on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum SettingsTab {
    /// No settings tab.
    #[default]
    Shut,
    /// The tab is open and a terminal is the tab on screen.
    Behind,
    /// The tab is open and is the one on screen.
    Shown,
}
```

Every gesture, and what it does:

| Gesture | `Shut` | `Behind` | `Shown` |
|---|---|---|---|
| The settings mark, or `Ctrl+Shift+Comma` | `Shown` (fresh scroll and focus) | `Shown` | `Shut`, and the terminal that was active is the tab on screen again |
| A click on the settings tab's cell | — | `Shown` | `Shown` |
| A click on a terminal tab, `Ctrl+Tab` | — | — | `Behind`, and that terminal is on screen |
| `Ctrl+Shift+T`, the `+` | unchanged | unchanged | a new terminal tab opens and is on screen; `Behind` |
| The close-tab chord, or a middle click on the settings cell | — | — | `Shut` |
| Escape, having already left the focused row | — | — | `Shut` |
| A right click on the settings cell | — | nothing | nothing |

The one that matters is the close-tab chord: today `Action::CloseTab` closes the *active terminal*,
and while the settings page is on screen the app's active terminal is still whatever it was — so a
chord that means "close what I am looking at" would kill a shell the user cannot see. While `Shown`
it closes the settings tab instead, and the app is not asked.

A right click on the settings cell opens no menu. The menu a tab has is "close", "rename" and the
splits; a page that has one name and three ways to close already has a menu with nothing in it, and
a menu that opens empty is worse than one that does not open. `Hit::Tab` is matched in three places
in the host — `open_tab_menu` (`host.rs:1545`), `close_tab` on a middle click (`host.rs:1556`) and
`activate` (`host.rs:1611`) — and each one destructures `TabId::Terminal(number)` for the number it
wants and routes `TabId::Settings` to the table above. `hover_target` (`lib.rs:1088`) and
`retarget` (`lib.rs:1165`) compare against `active`, which is a `TabId` too, and `Host::tabs`
(`host.rs:921`) is what builds one.

What changes around the frame:

- The grid is not drawn while `Shown`: `host.rs:818` draws it only when
  `!self.settings.shows()`. The content area is the page, and a terminal drawn under a page that
  covers it is a frame of work nobody sees.
- The scrollbar is not drawn while `Shown`: the host passes `ScrollState::default()` (visible 1.0,
  which draws no bar) for the same reason. `self.scroll` is untouched, so the terminal's position
  is where it was when the user comes back.
- `ChromeInput.active` is `Some(TabId::Settings)` while `Shown`, and the active terminal's number
  otherwise. `Host::tabs` appends `TabInfo { id: TabId::Settings, title: "settings", hovered }`
  when the tab is not `Shut`.
- The OS window title follows the tab that is on screen: `settings — zet` while `Shown`. The
  taskbar says what the window is showing, and a window showing settings that calls itself after a
  shell is a window that lies about it.
- Keys: `panel_key` still takes its six keys while `Shown`, and Escape still closes on the second
  press. What changes is the fall-through: a key that is neither a panel key nor a chord zet owns
  (`App::owns`, `app.rs:817`) is **swallowed** while `Shown`, rather than typed into a terminal that
  is not on screen. That is a deliberate reversal of the panel's rule — DESIGN.md's "the terminal
  behind the panel is live" was true because the terminal was visible and it is what made the panel
  an overlay; a page has nothing behind it. Chords still work: `Ctrl+Shift+T`, `Ctrl+Shift+W`,
  the settings chord and quit all reach the app.
- The wheel: `over_panel` keeps its logic and its name changes to `over_page`; the region it reads
  is the page's, which now covers the content rect.
- Mouse, which the panel never had to think about: while `Shown`, `forward_motion`
  (`host.rs:1412`) and `forward_mouse` (`host.rs:1591`) are not called, so a program that asked for
  mouse reports stops receiving them for a page it cannot see — and `grid_cell` (`host.rs:1753`) is
  not consulted, because the point it would resolve is over the page. Nothing is lost: the program's
  mouse mode is the program's, `self.app.active()` still names it, and forwarding resumes the moment
  a terminal is the tab on screen. `wheel_to_program` (`host.rs:1856`) is already behind
  `over_page`, which now answers for the whole content rect.

## What is deliberately not here

- **No visible `×` on the settings tab.** The strip has no close affordance anywhere, and one tab
  wearing one would be a tab that looks different for a reason the user cannot see. The mark, the
  chord and the middle click are the three ways out.
- **No second settings tab, and no way to have two.** Settings are one page; a second copy of it is
  a second view of one file.
- **No reordering and no dragging of the settings tab.** It is always last, which is where a page
  that is not part of the run belongs.
- **No pane splitting, no docking, no resize handles.** The page is the content area and the
  terminal is a tab, which is the whole of what was asked.
- **No change to what a setting is or does.** The rows, the ids, the effects, the capture, the save
  and the keyboard walk are all untouched; the page draws them where the panel did.
- **No relayout of the grid.** `Layout.grid` is the same rect as before and the PTY is resized by
  the same numbers; while the page is up nothing asks the sessions for a new size, because nothing
  about the grid changed.

## Files

- `crates/zet-ui/src/lib.rs` — `TabId`, `TabInfo.id`, `Hit::Tab(TabId)`, `Region::Tab { id }`,
  `ChromeInput.active: Option<TabId>`, `settings_open` removed, the page drawn from `plan`, the
  panel's slide state deleted.
- `crates/zet-ui/src/strip.rs` — the numberless cell, `number_cell(TabId)`, `Travel`/`TabCell` keyed
  by `TabId`, `SETTINGS_CELL` in the layout, the two glows removed, the settings mark's state.
- `crates/zet-ui/src/overlays.rs` — `panel` becomes `page`, the rail and the heading lose
  `.to_uppercase()`, the slide and the width clamp go.
- `crates/zet-ui/src/marks.rs` — `glow` and its three constants deleted.
- `crates/zet-ui/src/geometry.rs` — `SETTINGS_CELL` in, `PANEL_WIDTH`, `PANEL_SLIDE_MS`,
  `PANEL_SLIDE` out.
- `crates/zet-ui/src/tests.rs` — see the list below.
- `crates/zet/src/host.rs` — `SettingsTab`, the gesture table, the grid and scrollbar gates, the
  mouse gates, the settings tab in `Host::tabs`, the title, the swallowed keys, and the four host
  tests that assign `settings_open`.
- `DESIGN.md` — the settings section, the motion section, the rail's heading rule, the two marks'
  hover, the settings control's width, the palette rules if the glow's rings are named there.
- `specs/2026-09-27-hover-motion.md` — the glow it specifies is superseded, and the third authored
  moment is gone.
- `specs/2026-09-27-settings-rail-and-menu-layering.md` — it states the reverse of this plan
  ("The panel keeps overlaying the terminal: it does not dock, does not resize the grid"), and it
  is a record of a decision that has been reversed. It gets a note saying so rather than being
  rewritten: the spec is what was decided then.
- `CHANGELOG.md` — the `[Unreleased]` entries that describe the panel ("the settings panel is
  opened by a control of its own", "The panel is 560 pixels wide", "the one thing in the strip
  drawn over the panel it opens") describe a thing that will not exist, and are replaced before
  this is released. The `[Unreleased]` section is where they live, so they are rewritten rather
  than deleted.

## The tests

**The inventory below is the point of this section. The audit found that an earlier draft named
thirteen tests; the real number is fifty-odd, because `settings_open` is not one test's business —
it is how every settings test says "the panel is up". Implementing this plan is mostly rewriting
tests, and a plan that lists a third of them is a plan that leaves the workspace not compiling.**

The mechanical rewrite, which covers most of them. `ChromeInput.settings_open: bool` is replaced by
`active: Option<TabId>` and by the presence of a settings cell in `tabs`, so every site becomes two
lines:

```rust
input.active = Some(TabId::Settings);
input.tabs.push(TabInfo { id: TabId::Settings, title: "settings".into(), hovered: false });
```

which is what the new helper is for. The zet-ui sites that assign `input.settings_open` directly —
not through a helper — are at `tests.rs:932` (`the_grid_gets_what_is_left`, inside the test that
establishes the grid rect), `:1064`, `:1089`, `:1385`, `:1465`, `:1472`, `:1486`, `:1512`, `:1564`,
`:1648`, `:1702`, `:1729`, `:1833`, `:2239`, `:2379` (inside
`the_chrome_only_ever_draws_the_chrome_palette`, which loops over settings-open frames — the test
that guards the two-plane rule has to keep seeing a settings frame, now a page), `:2393`, `:3068`,
`:3381` and `:3660`.

Deleted, because the thing they test is gone:

- `the_panel_slides_in_over_the_time_the_document_gives_it` (`:1641`)
- `reduce_motion_puts_the_panel_in_place_on_the_frame_it_opens` (`:1695`)
- `closing_the_panel_is_instant_and_reopening_slides_again` (`:1722`)
- `the_new_tab_mark_and_the_settings_mark_glow_under_the_pointer` and
  `a_glow_stays_inside_the_control_that_owns_it` — replaced by item 6 below.

Deleted, and the one deletion with a cost: `open_panel`, `open_panel_at`, `open_panel_on`
(`:1047-1100`), `panel_rect_of` (`:1614`) and `panel_chrome` (`:1627`). These are helpers, so
deleting them orphans their callers — `panel_rect_of` is called from `:3665`, `:3775`, `:3813`,
`:3840` and `:3876` as well as from the three slide tests, and every one of those has to move to the
new helper in the same change.

Rewritten, because the answer they check changed:

- `the_settings_panel_is_anchored_to_the_right_and_hit_tests_as_settings` (`:965`) →
  `the_settings_page_is_the_content_area...`: the page's rect is the grid rect, and a point in it
  answers `Hit::Settings`.
- `a_section_heading_is_drawn_upper_case` (`:1774`) → `a_page_heading_is_drawn_as_the_caller
  _wrote_it`, asserting `Appearance` is drawn and `APPEARANCE` is not.
- `a_heading_scrolled_off_the_panel_takes_its_rule_with_it` (`:1808`) — same mechanism, a page.
- `the_settings_control_sits_beside_the_new_tab_mark` (`:3555`) — the width is `SETTINGS_CELL`, not
  the plus's, and the two marks are 25.5px apart.
- `the_settings_control_is_the_cell_under_the_new_tab_mark_in_the_rail` (`:3626`) — the same, in
  the rail, where the strip is a row of the rail rather than a row of the window.
- `the_settings_control_is_dropped_before_the_new_tab_mark_is` (`:3598`) — the window it is dropped
  in is 19px narrower.
- `the_settings_control_is_drawn_over_the_panel` (`:3654`) — the assertion becomes that the mark is
  *not* drawn over the page: the page ends where the strip begins.
- `the_scrollbar_is_still_reachable_with_the_settings_panel_open` (`:2217`) and
  `a_popover_row_is_still_reachable_with_the_settings_panel_open_behind_it` (`:3042`) — the page
  covers the content rect, so what these assert about an overlay is now about a page.
- The other settings tests — `the_panel_draws_the_lines_it_is_given` (`:1173`), `the_hover_fill
  _covers_the_region_a_click_would_take` (`:1213`), `every_control_is_a_region_that_names_its_line`
  (`:1261`), `a_stepper_answers_twice_and_everything_else_once` (`:1300`), `clicking_the_panel_is
  _not_clicking_the_grid` (`:1886`), and the group at `:1364`, `:1424`, `:1494`, `:1545`, `:1914`,
  `:2375`, `:3705`, `:3762`, `:3801`, `:3835`, `:3860`, `:3893`, `:3912` — keep their assertions
  and change how they say the panel is up. "Clicking the panel is not clicking the grid" is
  unchanged in meaning and, now that the page *is* the grid rect, unchanged in the code that proves
  it.

In `zet`, four tests assign `host.settings_open = true` and have to say `SettingsTab::Behind` or
`Shown` instead: `the_panel_hands_the_keyboard_back_once_tab_has_walked_off_the_end` (`:2447`),
`clicking_a_row_takes_the_keyboard_back_from_the_shell` (`:2497`), `walking_past_a_sections_last_row
_brings_the_next_one_to_the_page` (`:2615`) and `clicking_a_section_shows_it_and_gives_the_keyboard
_back` (`:2653`).

New in zet-ui:

1. `the_settings_tab_is_the_last_cell_and_has_no_number` — the cell is after every terminal cell,
   its width is padding plus the title, and no `#` is drawn in it.
2. `a_click_on_the_settings_tab_is_the_settings_tab` — `Hit::Tab(TabId::Settings)` in its middle,
   and `Hit::NewTab` is still the mark beside it.
3. `the_settings_page_fills_the_content_area_and_not_the_window` — the page's published surface is
   the grid rect: it starts below the strip and stops at the window's right edge.
4. `the_page_is_the_active_tab_and_the_terminals_are_not_drawn_under_it` — with
   `active = Some(TabId::Settings)`, no tab cell is drawn in the heavy weight and the settings rows
   are.
5. `a_settings_cell_is_sixteen_pixels_and_the_marks_are_twenty_five_apart` — the numbers, so that a
   later change to either cell has to be a decision.
6. `the_plus_and_the_settings_mark_draw_nothing_behind_themselves` — the frame with each hovered
   has, inside the control's rectangle, exactly the mark's own rectangles: nothing wider than
   `MARK_BOX`, and every one of them a stroke of the mark.
7. `the_settings_mark_is_ink_while_the_tab_is_open` — the mark's colour with and without a
   `TabId::Settings` in the input's tabs.

In `zet`:

8. `the_settings_tab_opens_shows_and_closes` — the mark's three states.
9. `the_close_chord_closes_the_settings_tab_and_not_the_shell_behind_it` — the chord while `Shown`
   leaves the shell that was behind it alive.
10. `the_settings_follows_the_active_tab` — the tab stays in the strip while a terminal is on
    screen, and the mark stays lit.
11. `a_key_with_no_binding_does_not_reach_the_shell_while_the_page_is_up`, and a chord does.
12. `the_mouse_is_not_forwarded_to_a_shell_that_is_not_on_screen` — a program in reporting mode
    gets nothing while the page is up, and gets its reports again once it is not.

## Risks

- **The `TabId` rename is wide.** It touches every place a tab's identity is compared, keyed or
  published. The compiler finds them all, which is the argument for doing it as a rename rather
  than as a second field that means the same thing.
- **The host's key path is subtle.** Swallowing an unbound key while `Shown` is a behaviour change
  in the one place where a mistake types into a shell the user cannot see. The test above is the
  guard.
- **`ChromeInput.settings_open` is removed**, which is a breaking change to a public struct in a
  crate the app owns outright — no other consumer, so there is nothing to shim.

## What the review changed

An adversarial pass read this plan against the code before any of it was written. It found four
things that would have cost real time and one that would have cost a wrong build.

1. **The test inventory was a third of the real set.** The draft named thirteen tests. The audit
   listed fifty-odd, including eighteen in `zet-ui` that assign `input.settings_open` directly and
   four in the host that assign `host.settings_open`, and it found that deleting `panel_rect_of`
   orphans five callers beyond the three slide tests. The section above is rewritten from that list.
   This was the finding that mattered: the request as drafted would not have compiled.
2. **`number_cell` answering `0.0` was arithmetic that did not produce the cell it promised.**
   `tab_width` is `floor(number_cell) + TAB_GAP + advance(title)`, so a zero number cell would have
   given a cell with no padding and a gap it should not have. The unit is `2.0 * TAB_PADDING` and
   the gap is skipped for a settings cell.
3. **The claim that the page would be drawn from a different pass than the panel was false, and
   the risk it was guarding against does not exist.** `plan` calls `overdraw` (`lib.rs:874`);
   they are one pass, and the host draws the grid before the chrome at all (`host.rs:830` then
   `:846`). The page keeps the same call site and the same order.
4. **The mouse was unaddressed.** The plan gated the grid, the scrollbar and the keys, and said the
   page "has nothing behind it" while `forward_motion`, `forward_mouse`, `wheel_to_program` and
   `grid_cell` all still went to the hidden shell. Now gated, with a test.
5. **Two documents were missing from the list**: the rail spec that states the opposite decision,
   and the `[Unreleased]` changelog entries that describe the panel.

Six line numbers were off by a few lines each and are corrected above — `ChromeInput.active` is
`:101` and not `:100`, the settings mark is drawn at `:935` and not `:930`, the glow inside
`settings_mark` is at `strip.rs:336` and not `:333`, `tab_number` is at `strip.rs:610` and not
`:607`, `TabCell` is at `strip.rs:50` and not `:47`, `PANEL_SLIDE` is at `geometry.rs:203` and not
`:200`, the host's settings fields run to `:353` and not `:347`, and the fall-through quote is at
`host.rs:1114` and not `:1112`. The audit confirmed the rest of the citations, and confirmed that
`App::owns` (`app.rs:816`) is reachable exactly where the swallow has to go.

## What was built

Everything in the plan, with the deviations below. `cargo test --workspace` is green — 32 suites,
`zet-ui`'s 112 and `zet`'s 107 among them — and `cargo clippy --workspace --all-targets -- -D
warnings` and `cargo fmt --all -- --check` are both clean.

Built as planned: `TabId` and the `TabInfo.id` threading, the removal of `settings_open` in favour
of `active: Option<TabId>` and the presence of a settings cell in `tabs`, the three-state
`SettingsTab` in the host, the page taking the content area, `PANEL_WIDTH`/`PANEL_SLIDE_MS` and the
whole slide apparatus deleted, `marks::glow` and its constants deleted, both `.to_uppercase()` calls
removed, the settings control at a fixed 16 pixels, and the four interception rules in the host's
key path.

Deviations, all deliberate:

- **The settings mark is `ink` whenever the settings tab exists, not only after a hover.** The plan
  had the mark lit by the pointer like the captions. The strip's own rule — the active tab's number
  is `ink` and the rest are `ink-mid` — is what a lit mark means here, and the tab appearing in the
  run with the mark dark would be a tab whose handle did not say it was open. Under the pointer a
  lit mark simply has no second look to give.
- **`Host::key` was split, and the page's rules moved into `Host::page_key`.** The plan's tests 9 to
  11 wanted the chord interception, the swallow and the ordering asserted together, and none of
  them is reachable from a test while the decisions are inline in a function that needs an
  `ActiveEventLoop`. `page_key` is exactly the block that was there, in the same order, returning
  whether it took the key; `key` calls it and returns. The one thing this costs is that the
  `Settings` chord and the close chord are now in a function named for the page — which is what
  they act on.
- **`Host::tab_press` was extracted from `chrome_press`**, which the two new tab arms had pushed
  one line over clippy's hundred. It is the two arms and nothing else.
- **`Host::mouse_reaches_the_shell` is a method rather than an inline guard.** Test 12 asks about
  three states, and `send_mouse` cannot be called from a test — it writes to a pty. The predicate is
  the guard `send_mouse` now uses, so the three states are assertable and the rule is stated once.
- **`a_settings_cell_is_sixteen_pixels_and_the_marks_are_twenty_five_apart` is not a test.** Both
  numbers are asserted inside `the_settings_control_sits_beside_the_new_tab_mark`, which is the
  test about the pair's geometry, and the 16 is asserted a third way in
  `the_settings_control_is_dropped_before_the_new_tab_mark_is`, whose threshold is derived from
  `SETTINGS_CELL`. A fourth copy of the same two literals would be a test that could only fail when
  three others already had.
- **`the_settings_control_is_drawn_over_the_panel` keeps its name and its assertion.** The page ends
  where the strip begins, so the mark is not drawn over it — but the mark is still drawn after the
  page's surface, which is what the test's push-order assertion is about, and the press at the
  mark's centre still has to answer `Hit::SettingsButton`.
- **Five tests were deleted, not four.** The plan named the three slide tests and the glow test; the
  second glow test (`a_glow_stays_inside_the_control_that_owns_it`) came with it, rewritten as
  `a_hovered_mark_reaches_no_further_than_the_control_that_owns_it` for ink that does not leave its
  cell.
- **One fixture changed in `the_settings_control_is_dropped_before_the_new_tab_mark_is`.** The
  original narrow-window arithmetic no longer describes the layout: the pair reserves its room in
  `horizontal` before the run of tabs is sized, so the controls only ever compete in a strip with no
  room for a tab at all. The test is now about that strip, and it says so.
- **`a_crowded_strip_squeezes_every_tab_by_the_same_amount` is eleven tabs, not twelve.** The
  settings control holds sixteen pixels of the row whether or not the pointer is on it, and the
  twelfth tab is what those pixels cost.

Verification, in the order it was run: `cargo test --workspace` (32 suites green),
`cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo fmt --all -- --check`
(clean), then the release build and the installer.
