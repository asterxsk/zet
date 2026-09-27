# The close mark on a tab, the last tab that does not end the window, and motion for the
# surfaces that arrive

**2026-09-27.** Spec and implementation plan for one change to the chrome: every tab gets an ×
that closes it, in both positions of the strip, the window stops ending when the last terminal goes
while the settings page is open, and the surfaces that appear over the screen ease in instead of
cutting.

Status: planned. Revised after review — see "what the review changed" at the end.

## The request

"I needed to add some smooth animations for all the elements and also fix the bug where the tabs
and the settings tab don't have an X on the right of the tab to close the tab. So just add that.
Additionally, closing the first tab should not close the app if settings is open. If there are no
tabs including settings that are open actively, only then should the app close."

Three things:

1. A close mark on every tab, the settings tab included, at the right of the cell.
2. Closing a terminal while the settings tab exists does not end the window. The window ends when
   there is nothing left in it: no terminal and no settings tab.
3. Motion, for the elements that have none.

Two of the three contradict something `DESIGN.md` states as a rule, so the rules change with them.
Both are called out rather than quietly overridden.

## What is true today

### The strip has no close mark, and the plumbing for one is half-built

- `Hit::CloseTab(u32)` exists (`crates/zet-ui/src/lib.rs:380-387`) and says in its own doc that it
  "is never returned by this version", because "DESIGN.md's strip is a number, a bar, and nothing
  else, and its hover rule is exhaustive ... so a close mark on a tab would be an invention this
  crate is not entitled to make". The variant was kept so a caller matching on `Hit` would not have
  to be rewritten when the convention became an affordance. It is now.
- Nothing in the workspace constructs `Hit::CloseTab`, and there is no `Region::CloseTab`
  (`lib.rs:513-549`), so nothing can be hit. The one arm that consumes it is
  `crates/zet/src/host.rs:1869-1872`, which calls `self.app.close_tab(number)` and throws the answer
  away. That arm is reached from `Host::button` (`host.rs:1600`), which calls `chrome_press` at
  `host.rs:1652` — there is no `Host::mouse_press`.
- **A tab's cell already ends its name twelve pixels short of its right edge.** `tab_width` is
  `floor(number_cell) + TAB_GAP + advance(title)`, capped (`crates/zet-ui/src/strip.rs:408-417`);
  `number_cell` is the number plus `2 * TAB_PADDING` (`strip.rs:387-392`); the name's room is
  `width - number_cell - number_gap` (`strip.rs:204`); the name is drawn from
  `cell.x + TAB_PADDING` after the number and its `TAB_GAP` (`strip.rs:636-657`, `:665-687`). So the
  name's right edge lands at `cell.x + width - 12` in **every** cell: the right `TAB_PADDING` is
  always empty. `TAB_PADDING` is 12 (`crates/zet-ui/src/geometry.rs:135`) and a mark's box is 10
  (`crates/zet-ui/src/marks.rs:47`).
- A cell is `TabCell { id, rect, title }` (`strip.rs:44-54`), where `title` is how many characters
  of the name fit. Cells are planned left to right in `horizontal` (`strip.rs:193-211`) and
  published in `Chrome::publish` (`lib.rs:1011-1016`) in forward order, where `Region::hit` answers
  with the **first** region holding the point (`lib.rs:553-581`, `find_map` at `lib.rs:780-784`).
  `Region::hit` ends in `_ => None` (`lib.rs:579`), so a variant added to `Region` and not to that
  match compiles and never answers.
- The rail's cells are `RAIL_WIDTH` = 48 (`geometry.rs:171`) by a compressed cell height, and the
  number is **centred** in them (`strip.rs:282-288`, `strip.rs:569-572`).
- A tab's `hovered` flag is filled by `Host::tabs` (`host.rs:989-1018`), which matches only
  `Hit::Tab(id)` (`host.rs:992-995`). `strip::hover_preview` (`strip.rs:783-801`) reads that flag.
- The chrome's hover target is one `Hit`, and `strip::draw` forces the *ink* hover to zero for the
  active tab while keeping the active tab's ink full (`strip.rs:549-557`).
- DESIGN.md: "It is closed by the mark, by `Escape`, by the close-tab chord, and by a middle-click
  on its cell, and there is no × on it — the strip's tabs have never had one."

### The window ends with its last terminal, and `Command::Quit` means two things

- `Command::Quit` is produced by three different things, and only one of them is "the last tab
  went":
  - `Action::CloseTab`, when `App::close_tab` answered `true` (`crates/zet-app/src/app.rs:937-945`).
  - The tab menu's `Close` on the last tab (`app.rs:615-619`).
  - `Action::Quit` (`app.rs:965`), which is a bound action with a default chord
    (`crates/zet-config/src/config.rs:457` `("quit", "Alt+F4")`) and is the user saying "close the
    window", not "close a tab".
- `Command`'s doc claims the command means "The last tab closed" (`app.rs:62-63`), which is already
  false for the third producer.
- `host.rs` has seven syntactic `loop_.exit()` sites; the doc at `host.rs:2343-2352` names five
  conceptually, two of which (`host.rs:2365`, `:2374`) are the startup-failure arms in `resumed`.
  Three of the five mean "the last tab went":
  - `Command::Quit` in `carry_out` (`host.rs:1129`).
  - A middle-click on the last terminal's cell (`host.rs:1718-1722`).
  - A shell exiting, in `user_event`: `if self.app.sessions().is_empty() { loop_.exit(); }`
    (`host.rs:2411-2415`).
  The other two are the window's own — `Caption::Close` (`host.rs:1816`) and `CloseRequested`
  (`host.rs:2420`) — and neither changes.
- `SettingsTab` is `Shut`, `Behind`, `Shown` (`host.rs:364-392`), where `open()` is "the tab
  exists". `Host::tabs` appends the settings cell when it does (`host.rs:1011-1017`), and the
  active tab is the page when the page is shown (`host.rs:782-786`).
- **`Behind` with no terminals is unguarded**: `Behind` means "a terminal is the tab on screen".
  Once the last terminal can close while the page is open, nothing stops the page being left behind
  nothing.
- **`Action::CloseTab` is intercepted while the page is shown** (`host.rs:1210-1214`), so the chord
  closes the page rather than the shell, and the app's own action never runs in that state.
- `App::blinks()` (`app.rs:372-375`) is `config.cursor.blink && !reduce_motion()`, with no reference
  to whether a session exists, and `host.rs:2524-2536` schedules a redraw every 530ms while it is
  true. Its only production caller is `host.rs:2524`.
- DESIGN.md's empty-strip note reasons from the same fact: "the state it would appear in is
  unreachable: `close-tab` quits when the last tab closes, so a window with no terminals is a window
  that is on its way out." The conclusion still holds; the reason stops being true.
- Every place else that could assume a terminal exists already guards: `grid_size`
  (`host.rs:1067-1082`), `scroll_state` (`host.rs:1022-1039`), `drag_scrollbar`, `wheel`,
  `os_window_title` (`host.rs:968-982`), `Host::tabs`, `Action::NextTab`/`PreviousTab`
  (`app.rs:946-958`) and the menu's `CloseOthers` (`app.rs:621-634`) all check `active()` or `get()`
  and do nothing with none. The two real exceptions are `blinks`, above, and the fact that the strip
  has no cell to middle-click.

### One thing in the chrome has motion; the rest cut

- `crates/zet-ui/src/hover.rs` is the whole mechanism: `Lit { hit, from, start, on }`, `Hover::step`
  with **one** target, `Hover::of` answering a coverage, `Hover::moving` as the frame clock.
  `hover.rs` has no test module of its own; its only caller is `lib.rs:741`, and `crates/zet-ui/src/tests.rs`
  drives a `Chrome` rather than a `Hover`.
- Two spans: `HOVER` = 110ms (`geometry.rs:206-209`) and `TRAVEL` = 140ms (`geometry.rs:165-168`),
  both on `strip::progress`, which is exponential ease-out (`strip.rs:810-818`).
- `Chrome::moving` (`lib.rs:803-805`) is what the host polls in `about_to_wait` to keep asking for
  frames (`host.rs:2511-2518`).
- Everything else is instant: the menu (`overlays::context_menu`, `overlays.rs:237`), the profile
  picker (`overlays::profile_picker`, `overlays.rs:118`), the find bar (`overlays::find_bar`,
  `overlays.rs:836`), the settings page, and the tab cells.
- DESIGN.md states the doctrine and two of its reasons, both in the Motion section: "**Everything
  else is instant**", "The settings tab opening and closing ... one that animates into existence
  would delay the actions inside it by exactly as long as the animation", "Tab open and close: the
  new tab appears already at full size. No scale-in, no slide", and, in the tab-menu section, "No
  timeout, no click-to-toggle, no fade. A menu that faded would be a menu where the click during the
  fade lands on the window underneath".
- `Painter::fill` takes no alpha (`crates/zet-ui/src/paint.rs:107-109`); `fill_at` does
  (`paint.rs:118`). Text is faded through `TextStyle::faded` (`paint.rs:48-51`).

## The design

### One: the × goes in the right padding a cell already has

**No new measurement, no reserved slot, and no cell changes width.** A mark is 10 pixels and every
cell already leaves its right 12 pixels empty — that is where the name is fitted to stop. The mark
is drawn centred in that padding, in both positions of the strip:

```rust
// strip.rs — on every cell, in `plan`, for both `horizontal` and `vertical`.
// The right padding is already there and already empty: a name is fitted to end
// `TAB_PADDING` short of the cell, so a mark inside the padding overlaps nothing and
// moves nothing, which is what lets it appear under the pointer without the run
// reflowing beneath it.
let close = tab.hovered.then(|| Rect::new(cell.right() - TAB_PADDING, cell.y, TAB_PADDING, cell.height));
```

The first plan reserved a 16-pixel slot in every cell and took it off every name. It is not needed:
the padding is the slot. What that buys, beyond the pixels:

- No cell's width changes, so no test of a cell's width, a name's fit, or the number of tabs that
  fit has a new fixture. `the_tabs_sit_side_by_side_and_a_cell_is_what_the_sum_of_its_parts_says`
  (`crates/zet-ui/src/tests.rs:371-399`) and `a_tab_with_no_name_is_still_a_tab`
  (`tests.rs:677-688`) both assert a cell's exact width and both keep passing.
- `tab_cap`'s floor and `TAB_MAX_WIDTH` are untouched, so a crowded strip behaves exactly as it did.
- The rail is not an exception. The rail's number is centred (`strip.rs:569-572`), so the rail
  reserves the same right `TAB_PADDING` and centres the number in `RAIL_WIDTH - TAB_PADDING`
  instead. The number moves six pixels left of centre in every rail cell, permanently, and both
  positions of the strip then carry the same mark in the same place. The alternative — no × in the
  rail — was rejected as the user asked for every tab: a rail cell is 48 pixels and the mark's 12
  fit beside a centred number only for single- and double-digit indices, and "it works until tab
  ten" is worse than six pixels.

The mark is drawn only while the pointer is over its cell, at that cell's own hover coverage, in
`ink_mid`; the pointer moving onto the mark itself draws it in `danger`, which is the colour the
caption's close button already spends and for the same reason — it is the control that takes
something away. That last step is a rectangle comparison inside the painter rather than a
transition, and it is the one place in the chrome where a hover is not a fade; the reason is that
the mark does not exist until the pointer is already inside its own cell, so the two states are one
control seen from two distances rather than two controls the pointer crosses between. It is
documented where it is written.

### Two: the pointer reaches the mark through the cell it is in

A control inside a control, which the hover mechanism has not had before. Four rules:

1. **`Hit::CloseTab(TabId)`, not `CloseTab(u32)`.** The settings tab has no number, and a variant
   that could only name the tabs that do is a variant the settings tab cannot use. The payload
   change forces the one existing arm (`host.rs:1869`) to be rewritten rather than silently
   matched.
2. **The mark's region is published before its cell's**, in `Chrome::publish`. `hit` answers with
   the first region holding the point, so the mark takes the pixels it occupies and the cell keeps
   every other pixel of itself, including the rest of its padding.
3. **`Chrome::hover_target` keeps the cell lit while the pointer is on the mark.** The chrome names
   every control the pointer is on: `Hit::CloseTab(id)` is arrived at from the tab's own cell, and
   the cell is still what the pointer is inside. Without this the cell's ink fades the moment the
   pointer reaches its own ×, which fades the × out from under the pointer.
4. **`Host::tabs` counts a pointer on the mark as hovering the tab** (`host.rs:992-995`), for the
   same reason and for `TabInfo::hovered`'s other reader, `strip::hover_preview`.

**The mark's region is published only while the mark is drawn.** A click through an invisible mark
would close a tab the user never aimed at, and a point in the right padding of a cell the pointer is
not over answers `Hit::Tab` exactly as it does today. The two-step this creates is stable rather
than delicate: the pointer arriving on the cell draws the mark and publishes its region, and rule 3
keeps the cell lit — and therefore the region published — for as long as the pointer is on the mark.
Where the plan first had to thread the hover state into `Chrome::publish` to make this decision, the
plan's own `TabCell` carries it instead: `close: Option<Rect>`, decided in `strip::plan` from
`TabInfo::hovered`, which the caller already answers from `Chrome::hit`. `publish` reads the plan it
is already handed and its signature does not change.

### Three: the window ends when there is nothing left in it

Two functions with one job each, because the plan first had one that both acted and answered and was
called from a door that closes nothing:

```rust
// host.rs
/// Nothing is left in the window: no terminal, and no settings tab either.
///
/// The one question the window's ending is a function of. `close-tab` used to end the
/// process the moment the last shell went; the settings tab outlives the last shell, so
/// the window does too, and the question is not "is there a tab" but "is there anything".
fn nothing_left(&self) -> bool {
    self.app.sessions().is_empty() && !self.settings_tab.open()
}

/// Bring the settings page to the front when it is now the only tab.
///
/// `Behind` means "a terminal is the tab on screen". A page behind nothing is a page the
/// user cannot see on a window that is still in front of them, so the last terminal
/// closing hands the screen to the page.
fn bring_the_page_forward(&mut self) {
    if self.app.sessions().is_empty() && self.settings_tab.open() {
        self.settings_tab = SettingsTab::Shown;
        self.redraw();
    }
}
```

**`Command::LastTabClosed` is a new command and `Command::Quit` stops meaning it.** Gating
`Command::Quit` on `nothing_left()` would have broken the user's own quit: `Action::Quit` produces
`Command::Quit` (`app.rs:965`) with a terminal open, `nothing_left()` answers false, and the window
would refuse to close. So:

- `Command::Quit` keeps its doc — "the window is to close" — and `carry_out` exits unconditionally
  (`host.rs:1129`).
- `Command::LastTabClosed` is new, means "the app's last tab went, and the host decides", and is
  produced by the two places that mean it: `Action::CloseTab`'s `Ok(true)` (`app.rs:942`) and the
  tab menu's `Close` on the last tab (`app.rs:618`). `carry_out` answers it with
  `bring_the_page_forward()` then `if nothing_left() { loop_.exit() }`.

The doors, and what each does. Every one of them ends in the same pair of calls:

| Door | Site | Acts |
|---|---|---|
| The close chord, the tab menu's `Close` | `carry_out`, `Command::LastTabClosed` | bring forward, then end if nothing |
| The middle click on a cell | `host.rs:1718-1722` | same |
| The new × | `chrome_press`'s `Hit::CloseTab` arm | same |
| A shell exiting | `user_event`, `host.rs:2411-2415` | same |
| The close chord or `Escape` while the page shows | `page_key` → its caller in `Host::key` | end if nothing |
| The settings mark, or `toggle_settings` | `chrome_press`'s `Hit::SettingsButton` arm | end if nothing |

The middle click and the × are the same act on the same cell, so they are one method, which is also
what makes the × reachable from a test — `chrome_press` returns early without a window
(`host.rs:1785-1787`), and this is the same reason `Host::page_key`, `Host::tab_press` and
`Host::mouse_reaches_the_shell` exist:

```rust
/// Close the tab a gesture was aimed at, and report whether the window has anything left.
///
/// The × on a cell and a middle click on one are the same act: close *that* tab, which is
/// not always the active one, and let the window decide what is left. A settings cell closes
/// the page instead of a shell, and the page has no `close_tab` to answer for it.
fn close_tab_from_strip(&mut self, id: TabId) -> bool {
    match id {
        TabId::Terminal(number) => { let _ = self.app.close_tab(number); }
        TabId::Settings => self.close_settings(),
    }
    self.bring_the_page_forward();
    self.nothing_left()
}
```

Four states this makes reachable that were not, and what each does:

| Terminals | Settings tab | What happens |
|---|---|---|
| 0 | `Shut` | The window ends. The only ending the app had before. |
| 0 | `Behind` | The page comes to the front; the window stays up. |
| 0 | `Shown` | Nothing moves; the window stays up. |
| 0 | `Shown`, then the page is closed | The window ends. |

**The empty strip stays unreachable, and now it is unreachable by construction rather than by
timing.** The close-chord path is the one that could have drawn it: `Host::key` requests a redraw
for the action (`host.rs:1176`) *before* `carry_out` reaches the quit, so a redraw can be queued in
the same event that ends the loop, and whether winit services it before stopping is the backend's
business. So the host gets one bool — `closing`, set wherever it tells the loop to end — and
`redraw` returns at once when it is set. The last frame a window draws is a frame with something in
it. It costs one field and one guard, and it is the reason the paragraph below is a fact.

DESIGN.md's empty-strip paragraph therefore keeps its conclusion and changes its reason: it is not
that "`close-tab` quits when the last tab closes" but that the window's ending and the strip's
emptiness are the same event.

The window has a state it never had — no shells, and a page of settings — and the rest of the host
already behaves in it: `grid_size`, `scroll_state`, `wheel`, the window title, `Host::tabs`,
`NextTab`/`PreviousTab` and the menu's `CloseOthers` all guard on `active()` or `get()` and do
nothing when there is nothing. The strip draws the settings cell and nothing else, `Strip::row`
stays true, and `Layout::top` stays 40.

One real defect is created by the state being permanent, and it is fixed here: `App::blinks()`
(`app.rs:372-375`) is true whenever the cursor blinks and motion is not reduced, with no reference
to a session, so `about_to_wait` (`host.rs:2524-2536`) redraws the window every 530ms for a cursor
that does not exist. A window with no terminals is a window with no cursor, so `blinks()` gains
`self.sessions.active().is_some() &&`. Today that wake is wasted on every frame the page is up;
after this change it is the terminal state of the app, so it is fixed with it.

### Four: what arrives over the screen eases; what replaces it does not; nothing leaves gradually

The rule, and the distinction is the repo's own — DESIGN.md's settings section is built on the
difference between an overlay and a tab:

> **A surface that appears over what is on screen fades in. A surface that replaces what is on
> screen — the settings page — is there on the frame it opens. Nothing that moves the layout is
> animated, nothing leaves gradually, and the grid is never animated.**

Arrive over, rather than arrive: the menu, the profile picker, the find bar, and a tab's own cell.
Not the settings page, and the reason is DESIGN.md's, kept: the page is a change of *what is on
screen* rather than a thing appearing over it, it is what the user asked to see, and fading it in
means 120ms in which the rows the user opened the page for are drawn at a fifth of their ink —
while their keyboard focus, which the arrow keys are already moving, is invisible. That is the one
place a fade would be in front of the user's own input, and it does not happen. DESIGN.md's
paragraph on the page survives unamended, which is the point: this change does not need to overturn
it.

**Nothing leaves gradually, and the reason is not that it would be impossible.** Two reasons, in
order of weight:

- **A departing tab's ghost overlaps the tabs that renumber into its place.** A tab's number is its
  position and closing one closes the run up (`DESIGN.md:152-157`), so a ghost of `#2` fading at its
  old rectangle sits on top of the `#2` that has taken that rectangle, and of the `#1` that has
  moved into the space to its left. The user sees two tabs' ink in one cell for 120ms, at exactly
  the moment the strip is telling them which tab is where.
- **A departing surface is a click hazard, and DESIGN.md already named it.** "A menu that faded
  would be a menu where the click during the fade lands on the window underneath"
  (`DESIGN.md:270-271`). A fade-*out* is that hazard: either the menu keeps its regions while it is
  visibly dying — so a click on a menu the user is watching dissolve hits it — or it drops them and
  a click meant for it falls through to the window. A fade is therefore allowed in *one* direction,
  which is why the rule above is about arriving.

The mechanism is the one that is already there, generalized by its key — one type, one `step`, no
second method (an inherent `step` and a type alias for the same type is a duplicate definition, and
`hover.rs` has no tests of its own to keep a wrapper for):

```rust
// hover.rs — `Lit` gains a key type parameter and loses the `hit`-specific name.
struct Lit<K> { key: K, from: f32, start: f32, on: bool }
pub(crate) struct Fades<K> { entries: Vec<Lit<K>>, now: f32 }

impl<K: Copy + PartialEq> Fades<K> {
    pub(crate) fn of(&self, key: K) -> f32;
    pub(crate) fn step(&mut self, now: f32, present: &[K], instant: bool);
    pub(crate) fn moving(&self) -> bool;
}
pub(crate) type Hover = Fades<Hit>;
```

Three facts make this the same object rather than a new one: `Chrome::hit` answers with one control,
so `Hover`'s slice normally holds one element; a key that is not in `present` fades out and is
dropped at zero, which is the whole lifetime rule; and `moving()` is unchanged, so `Chrome::moving`
picks up every new transition and the frame clock drives them without the host knowing they exist.
`Hit::None` is the empty slice, which is what `hover_target` already returns for a pointer over
nothing.

One span in `geometry.rs`, in milliseconds and seconds like the other two:

```rust
/// How long a surface that appears over the screen takes to arrive.
///
/// Between the hover's 110 and the indicator's 140: a surface is bigger than a control and
/// smaller than a journey, and a window with two of them arriving at once should not read as
/// a slow window.
pub const APPEAR_MS: f32 = 120.0;
pub const APPEAR: f32 = APPEAR_MS / 1000.0;
```

What gets it, and what each coverage multiplies. `Painter::fill` has no alpha
(`paint.rs:107-109`), so every fill named below becomes `fill_at`, and every text gets
`TextStyle::faded`:

| Element | Key | Every emission the coverage multiplies |
|---|---|---|
| A tab's cell — its number, its name, and its × | `TabId` | the cell's two ink cross-fades (`strip.rs:577-604`), the `+`'s pair (`:607-622`), and the mark's own coverage |
| The context menu | `Surface::Menu` | its surface, its **border** (`overlays.rs:248`), each row's fill, and the labels |
| The profile picker | `Surface::Picker` | the same three over its own rectangle |
| The find bar | `Surface::Find` | its **hairline** (`:840`), its **field** (`:850`), the field's **border** (`:851`), the **caret** (`:866`), the label, the query, and the count |

`Surface` is a small enum in `lib.rs`, one variant per overlay that fades. It is its own key type
because neither popover has a stable `Hit`: `Hit::Menu` and `Hit::Picker` carry rectangles that move
with the pointer, and a fade keyed by a rectangle restarts every time the menu is re-placed.

The tabs' presence set is the ids the caller handed over this frame; the chrome already sees the
previous frame's `input.tabs`, so a tab that was not there last frame starts at zero. A tab that
closes takes its id out of the set and its ink goes on that frame — which is also what the
renumbering needs, because a tab's number *is* its position: closing `#1` of three leaves
`Terminal(1)` and `Terminal(2)` present with different text, and a fade keyed on anything but the
position would fight the strip's rule that numbering closes up behind a tab that goes.
`Host::tabs` appends the settings cell when the page exists (`host.rs:1011-1017`), so the page's tab
fades in through the same mechanism with no second path.

**The active tab's cell still fades in, and its ink rule is untouched.** `strip::draw` forces the
*ink* hover to zero for the active tab (`strip.rs:553-557`); that is a rule about ink, not about the
pointer, so the raw `hover.of(Hit::Tab(id))` is kept and used for the ×'s appearance, and the ×
appears on the active tab — including on the settings tab, which is active whenever the page is up.

**A tab's ink fade and its hover fade multiply.** A cell that has just arrived and is under the
pointer is drawn at `presence * hover`, so it is `ink_dim`→`ink_mid`→`ink` through one product. This
is the only place in the crate where two coverages are multiplied, and it is stated here so that it
is not discovered.

### Five: what this does to DESIGN.md

- **Motion.** Four authored spans; a paragraph stating that a surface appearing *over* the screen
  fades in and one that *replaces* it does not; a paragraph stating that nothing leaves gradually,
  with the click hazard as the reason rather than a claim that it is impossible; and the tab
  paragraph amended — the cell is at its final rectangle on its first frame and takes a click there,
  and its ink arrives over 120ms, so the old reason ("a tab that animates into existence delays
  input") stops being true and is replaced by the one that still holds. The settings-tab paragraph
  keeps its instant arrival and its reason.
- **The strip.** "There is no × on it" becomes the mark, where it sits (the cell's right padding, in
  both positions), when it is drawn, and the two inks. The rail's number is centred in what the mark
  leaves rather than in the whole cell.
- **The tab menu.** The "no fade" rule is scoped to its dismissal, with the fade-in named as allowed
  and its direction as the reason.
- **The empty strip.** Its conclusion stands; its reason becomes the shared event.
- **`CHANGELOG.md`.** Added for the mark and for the window's new ending; Changed for the motion.

### Six: the tests

`zet-ui`, in `crates/zet-ui/src/tests.rs`:

- `the_run_does_not_move_when_the_pointer_crosses_it` — the same input planned with the pointer on a
  tab, on its mark, and nowhere gives byte-identical cell rectangles. This is the test that fails if
  someone later makes the slot conditional.
- `a_close_mark_sits_in_the_padding_a_cell_already_has` — the mark's rect is the cell's right
  `TAB_PADDING` in both positions, and the cell's width is the same as it was with no pointer.
- `a_close_mark_is_hit_before_the_tab_it_is_in` — a point in the mark answers `Hit::CloseTab`, a
  point on the number answers `Hit::Tab`, for one cell.
- `a_close_mark_that_is_not_drawn_is_not_hit` — with the pointer off the strip, a point in a cell's
  right padding answers `Hit::Tab`.
- `the_settings_cell_has_a_close_mark_of_its_own` — a point in the settings cell's mark answers
  `Hit::CloseTab(TabId::Settings)`.
- `a_mark_under_the_pointer_keeps_its_cell_lit` — the hover set for a pointer on a mark leaves
  `of(Tab(id))` at 1.
- `a_hovered_tab_gets_a_bar_and_a_mark` — **replaces** `a_hovered_tab_gains_a_bar_and_nothing_else`
  (`tests.rs:877-927`), which asserts that a hover adds exactly one rectangle. It now adds the bar
  and the mark's strokes, so the test finds the bar by its colour rather than by being the only
  thing there, keeps every one of its existing assertions, and names the mark.
- `a_tab_that_has_arrived_fades_in_and_one_that_was_here_does_not` — a new id starts at 0 and lands
  at 1 after `APPEAR`; an id present in both frames is 1 throughout.
- `an_arriving_surface_fades_and_a_departing_one_is_gone` — the menu's coverage rises over `APPEAR`;
  the frame after it leaves the input it is 0 and draws nothing.
- `the_page_is_there_on_the_frame_it_opens` — the settings page's coverage is 1 on its first frame,
  asserted the way the arrival test asserts the menu's is 0.
- `the_frame_clock_runs_while_anything_is_arriving` — `moving()` is true mid-fade and false once
  every fade has landed, a tab that is merely present included.
- `nothing_fades_when_motion_is_reduced` — with `reduce_motion` set, an arriving tab is at 1 on its
  first frame.

`zet`'s tests, in `host.rs`'s test module:

- `closing_the_last_terminal_leaves_the_settings_tab_on_screen` — page `Shown`, one terminal, a
  close: `nothing_left()` is false and the page is still `Shown`.
- `the_page_comes_to_the_front_when_the_last_terminal_goes` — page `Behind`, one terminal, the
  terminal closes: `Shown`, and `nothing_left()` false.
- `closing_the_last_terminal_with_no_page_ends_the_window` — page `Shut`, one terminal: `true`.
- `closing_the_page_with_no_terminals_ends_the_window` — no terminals, page `Shown`: closing the
  page leaves `nothing_left()` true.
- `closing_the_page_with_a_terminal_behind_it_leaves_the_window` — `false`.
- `the_strip_closes_the_tab_it_is_given_and_not_the_active_one` — `close_tab_from_strip` on a
  non-active terminal closes that tab and leaves the active one alone, which is what the × and the
  middle click both mean.
- `the_strip_closes_the_page_when_the_page_is_the_tab` — `close_tab_from_strip(TabId::Settings)`
  closes the page and not the shell behind it.
- `a_pointer_on_a_close_mark_still_hovers_its_tab` — through `Host::tabs`, which is what fills
  `TabInfo::hovered`.
- `a_shell_that_exits_leaves_the_page_up` — the reap path does not end the window while the page is
  open, and brings the page forward.

`zet-app`:

- `a_window_with_no_terminals_does_not_blink` — `App::blinks()` is false with no session, whatever
  the configuration says. The two existing assertions at `app.rs:2176-2178` are re-read for whether
  their fixture has a session.

The five `loop_.exit()` doors are not assertable from a test — there is no loop — and no test claims
to. What is asserted is the predicate the doors share and the method they share, which is what
`Host::close_tab_from_strip`, `Host::nothing_left` and `Host::bring_the_page_forward` exist for, the
way `Host::page_key` exists for the page's own rules.

## What is deliberately not here

- **Animated layout, of any kind.** The find bar's height, the tab run's reflow, the page's scroll
  and the strip's geometry all change in one frame. Animating the find bar's height would resize
  every pty on every frame of the animation; animating the run's reflow would move hit regions under
  a pointer that is aiming at them.
- **A fade-out for anything.** Section four.
- **The settings page fading in.** Section four, and DESIGN.md's reason is kept rather than
  overturned.
- **Press states.** A control under a held button reading differently from a hovered one is a state
  rather than motion; it is not asked for and it is not here.
- **The scrollbar's thumb easing.** The thumb is a picture of a proportion and the content it
  describes does not ease. A bar that glides while the grid has already jumped lies for 120ms about
  where the viewport is.
- **Any change to what closes a tab.** The chord, the middle click, `Close other tabs` and a shell
  exiting all behave as they do; the × is a fifth way to do what four already did.

## Files

- `crates/zet-ui/src/geometry.rs` — `APPEAR_MS`, `APPEAR`.
- `crates/zet-ui/src/hover.rs` — `Lit<K>`, `Fades<K>`, `Hover` as an alias, `step` taking a slice.
- `crates/zet-ui/src/strip.rs` — `TabCell::close`, the mark's rect in `plan` for both positions, the
  rail's centring, `close_mark`, and the cell's presence coverage in `draw`.
- `crates/zet-ui/src/lib.rs` — `Hit::CloseTab(TabId)`, `Region::CloseTab` and its `hit` arm,
  `Surface`, the two new `Fades` instances and the `moving()` that reads all of them, the hover
  target set, the presence sets, and the coverage passed to each overlay.
- `crates/zet-ui/src/overlays.rs` — an `alpha` on `context_menu`, `profile_picker` and `find_bar`,
  multiplied into every fill, border, hairline and glyph each one emits, with `fill` becoming
  `fill_at` where it has to.
- `crates/zet-app/src/app.rs` — `Command::LastTabClosed` and its two producers, and `blinks()`.
- `crates/zet/src/host.rs` — `nothing_left`, `bring_the_page_forward`, `close_tab_from_strip`, the
  six doors, `closing`, and `Host::tabs`'s hover match.
- `DESIGN.md`, `CHANGELOG.md` — section five.

## Risks

- **A mark inside a cell is the first nested hit region in the crate**, and `Region::hit` ends in
  `_ => None` (`lib.rs:579`), so a missing arm compiles and never answers. The ordering test is what
  catches it.
- **`Hit::CloseTab`'s payload change** forces the one arm that matches it to be rewritten. Anything
  that reads a tab's number out of `Hit::CloseTab` would be a compile error rather than a silent
  wrong tab, which is the reason for the change.
- **`Hover::step`'s signature changes** to a slice and its only caller is `lib.rs:741`. No test
  calls it directly, so the change is mechanical.
- **One existing test changes meaning**: `a_hovered_tab_gains_a_bar_and_nothing_else`, whose whole
  point was that a hover adds exactly one rectangle. It is rewritten to keep its assertions and
  name the mark, and the rewrite is the place a mistake could hide, so it asserts the bar's colour
  and its row as it does today.
- **The rail's numbers move six pixels left.** Visible, deliberate, and the alternative was a strip
  that behaves differently in its two positions.
- **The window can now outlive every shell.** The host's no-terminal paths are enumerable and are
  enumerated above; `blinks()` was the one that was wrong.

## What the review changed

Two reviewers were sent the first version of this plan — one to verify every claim against the code,
one to attack the design. Both found the same three things first, and all three were right:

1. **`Command::Quit` means two different things and gating it would have broken the user's own
   quit.** `Action::Quit` produces it (`app.rs:965`) with a terminal open, so `nothing_left()` would
   have answered false and the window would have refused to close on Alt+F4. Split into
   `Command::LastTabClosed` and section three's door table.
2. **The × is itself a door, and the first plan's door list did not include it.** The new arm would
   have closed the last shell and left a strip with nothing in it — precisely the state
   DESIGN.md:207-220 says cannot happen. `Host::close_tab_from_strip` and its two callers.
3. **The 16-pixel slot was not needed.** A cell's name already stops `TAB_PADDING` short of its
   right edge, in every cell, so a 10-pixel mark fits in padding that is already empty, at zero
   pixels and zero test churn. Section one is rewritten around that, and the rail exception is gone
   with it — the same padding exists in the rail.
4. **The queued redraw on the close chord was unverified**, and the "no frame is ever drawn with an
   empty strip" claim rested on it. `Host::closing` and the guard in `redraw` make it true by
   construction instead.
5. **The settings page should not fade in.** Its rows are interactive under a keyboard focus the
   user is already moving, so a 120ms fade of the content area is the one fade that would be in
   front of the user's input — which is DESIGN.md's own reason for the page being instant, and it is
   now kept rather than overturned. The rule became "over the screen fades; replacing the screen
   does not", which is the repo's own overlay-versus-tab distinction.
6. **The menu's "no fade" rule needed answering, not ignoring.** Its hazard is a click during the
   fade landing on the window underneath, which is a fade-*out* hazard, so the rule is scoped to the
   dismissal and the fade-in is named as allowed.
7. **`after_a_tab_goes` both acted and answered, and was called from a door that closes no tab**
   (`page_key` returns true for arrow keys). Split into `nothing_left` and `bring_the_page_forward`.
8. **`Hover::step`'s one-target wrapper was not implementable** — an inherent `step` beside an alias
   for the same type is a duplicate definition — and the justification for keeping it was false:
   `hover.rs` has no tests. One `step`, taking a slice.
9. **`App::blinks()` ignores whether a session exists**, so a window with no terminals redraws every
   530ms for a cursor that is not there. Fixed here, because this change makes that state permanent.
10. **`Chrome::publish` could not make the "not drawn ⇒ not hit" decision**, because the hover state
    is moved out before it is called. The decision moved into `strip::plan`, which already has
    `TabInfo::hovered`; `publish` reads `TabCell::close` and its signature is unchanged.
11. **Overlay alpha needed a complete inventory.** `Painter::fill` has no alpha, so every fill
    becomes `fill_at`, and the first plan's lists were missing the menu's border, the find bar's
    hairline, field, field border and caret.
12. Citations corrected: `Host::button`, not `Host::mouse_press`; cells are published front to back;
    `Lit`'s field is renamed from `hit` to `key`.

Rejected from the review, with the reason:

- **A tab that closes could be animated after all**, by the app carrying the departing title or the
  chrome owning one `String` per close. It is possible; it is still wrong, and the reason is the
  overlap in section four rather than the cost.
- **Fading a departing menu** because a fade is "not impossible". Section four.
- **Reserving the mark's slot only on the active tab.** It is exactly the reflow the slot was for.

## What was built

As designed, in three passes: `zet-ui`, then `zet-app`, then `zet`. The workspace is green —
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all
-- --check` — with `zet-ui` at 123 tests in-crate, `zet-app` at 147 plus 8 in `tests/`, and `zet` at
115 plus the two the `subsystem` integration test owns.

**The × is in `TabCell::close`, a `Option<Rect>`, and the mark's presence is the cell's hover.** So
the "not drawn ⇒ not hit" decision is `strip::plan`'s, where `TabInfo::hovered` already was, and
`Chrome::publish` reads the field without a new argument — which is what the review said was the only
place the decision could be made once the hover state is moved out first.

**One nesting rule the plan had not spelled out.** A pointer on the mark is a pointer on the cell as
well, and `hover_target` says so in the one arm that needed it: `Hit::CloseTab(id)` pushes
`Hit::Tab(id)` and then itself, so the cell keeps its hover and its previewed bar while the pointer is
inside its own ×, and the mark cannot fade out from under the hand reaching for it. It is the one
place in the chrome where a hit names two controls.

### Deviations from the plan

1. **`Host::close_tab_from_strip` is `Host::close_from_strip`.** It sits beside `close_settings`, and
   a name that said `tab` would read as a claim about the settings cell it exists to close.
2. **`Host::tabs`'s hover match became a free function, `hovered_tab`,** beside `next_focus`. The plan
   asked for a test that a pointer on a mark still hovers its tab; that test needs a laid-out chrome
   and `Host` has none, so the rule is a function of the hit and the test asserts the function. What
   is not covered by a test is the one line that feeds it, which is `self.pointer.and_then(...)`
   against `self.chrome.hit`.
3. **`a_pointer_on_a_close_mark_still_hovers_its_tab` is `a_pointer_on_a_close_mark_hovers_the_tab_it_closes`,**
   for the reason above, and it asserts all four hit shapes rather than the one.
4. **`a_shell_that_exits_leaves_the_page_up` is folded into
   `the_page_comes_to_the_front_when_the_last_terminal_goes`.** A real reap needs a real shell that
   exits on cue, and the door it goes through — `bring_the_page_forward` then `nothing_left` — is the
   same two calls the chord makes. The test names the reap door in its comment rather than pretending
   to press it.
5. **Three host tests the plan did not list, added because the module already had the fixture:**
   `a_window_that_outlives_its_last_shell_is_a_window_with_a_page_in_it` (the predicate on its own),
   and the two `the_strip_closes_the_page_when_the_page_is_the_tab` /
   `the_strip_closes_the_tab_it_is_given_and_not_the_active_one` pair, which is the payload change
   seen from the host's side rather than the chrome's.
6. **The middle-click arm matches `Hit::CloseTab` as well as `Hit::Tab`.** DESIGN.md says a middle
   click on the cell closes it, and a pointer over the cell's × is a pointer over the cell — an arm
   that matched only `Hit::Tab` would have made the one place a middle click did nothing be the mark
   that means close.
7. **The rail's label offset is a constant half-padding, not a re-measure.** `draw` centres the number
   in `number_cell(paint, id) - 2.0 * TAB_PADDING` when the strip is on the left and uses the plain
   left inset otherwise, so both positions put the × in the same place and the number's own
   measurement never moves.

### What this did not touch

The scrollbar's thumb still eases and the find bar's height still jumps in one frame, both
deliberately (see *What is deliberately not here*). `Painter::fill` still exists beside `fill_at`:
the chrome's opaque fills did not become fades, so they did not need an alpha they do not have.
