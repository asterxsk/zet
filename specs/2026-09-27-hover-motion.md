# Hover motion: an animated hover state for every control, caption backgrounds, and a glow

**2026-09-27.** Spec and implementation plan for one change to the chrome: hover becomes a
transition rather than a hard cut, and three controls grow a hover state they do not have.

Status: built. Revised after review — see "what the review changed" at the end, and "what was
built" after it.

## The request

"Add hover animations to all UI elements. For example, the window controls add proper colored
backgrounds, and for things like the plus button or settings button, just add a white glow when
hovered on."

Three things, and they are one mechanism and two new looks:

1. Every control that has a hover today gets one that eases in and out instead of snapping.
2. The three caption buttons get a background under the pointer — `danger` behind close, a wash of
   `ink` behind minimize and maximize.
3. The `+` and the settings mark get a hover at all (they have none today) and it is a white glow.

## What is true today

- Hover is a comparison against the pointer, made inside the painter, once per control:
  `input.pointer.is_some_and(|(x, y)| rect.contains(x, y))` at `strip.rs:311` (settings mark),
  `strip.rs:591` (captions), `overlays.rs:390` (rail), `overlays.rs:660` (panel controls),
  `overlays.rs:927` (scrollbar), `overlays.rs:242` (menu rows). Tabs are the exception: the app
  asks `Chrome::hit` and hands the answer in as `TabInfo::hovered` (`host.rs:897`).
- Every one of those flips a colour or a width immediately. Nothing eases.
- The `+` has no hover at all: `strip.rs:536` draws it in `ink_dim` unconditionally. The settings
  mark has one, and it is only a change of ink (`strip.rs:311`).
- The caption buttons have no background at all: `strip.rs:595-599` draws the mark in `ink_mid`,
  `ink` when hovered, `danger` when hovered and it is close.
- **A plain pointer move does not ask for a frame.** `Host::moved` (`host.rs:1381`) requests a
  redraw only while dragging a selection (`host.rs:1411`) or dragging the thumb. Hover feedback
  reaches the screen only when some other event happens to want a frame — the cursor blink at best,
  which is 530 ms. So today's hover states are already late, and an animation on top of them would
  be a slideshow.
- **Nothing drives a clock for the chrome.** The two transitions that exist — the indicator's
  140 ms travel and the panel's 180 ms slide — are advanced by whatever frames arrive; `about_to_wait`
  (`host.rs:2236`) wakes the loop for the frame hold and for the cursor blink and for nothing else.
  There is no "a transition is in flight, come back in 16 ms" path anywhere in the host.
- `Chrome` already owns both of the crate's transitions, and the crate's own rule is that the chrome
  never reads a clock: `Chrome::set_time` (`lib.rs:692`) is handed one, which is what makes a
  transition assertable seventy milliseconds into itself.
- The palette is opaque and fixed at twelve entries (`palette.rs:97`), and the crate's test
  `the_chrome_only_ever_draws_the_chrome_palette` (`tests.rs:2332`) asserts that every colour in
  every quad and every glyph is one of the twelve with its alpha divided back out. DESIGN.md already
  has the idiom for a colour at less than full strength: "selection in the chrome uses `signal` at
  20% alpha" (`DESIGN.md:518`).

## The design

### 1. One hover value per control, stepped by the chrome

`Chrome` gains a small list of what is lit, and each control asks it instead of comparing
rectangles itself.

New module `crates/zet-ui/src/hover.rs`:

```rust
/// One control's lit state and the transition carrying it there.
struct Lit { hit: Hit, from: f32, start: f32, on: bool }

pub(crate) struct Hover { entries: Vec<Lit>, now: f32 }

impl Hover {
    /// How lit `hit` is this frame: 0 at rest, 1 under the pointer.
    pub(crate) fn of(&self, hit: Hit) -> f32;
    /// Step every entry toward `target`, starting any transition that has to start.
    pub(crate) fn step(&mut self, now: f32, target: Hit, instant: bool);
    /// Whether anything is still moving.
    pub(crate) fn moving(&self) -> bool;
}
```

- The value is `from + (1 - from) * p` rising and `from * (1 - p)` falling, where
  `p = strip::progress(now - start, HOVER)` — the same exponential ease-out the indicator and the
  panel already use (`strip.rs:687`). The shape is not new; only the span is.
- **`Hit` is the key**, because `Hit` is already this crate's name for "a thing the user can point
  at" (`lib.rs:351`) and it is `Copy + Eq`. No new vocabulary, and a control that is hittable cannot
  be one that is unhoverable by accident.
- **One control is lit at a time**, and that is not an assumption: `Chrome::hit` answers with the
  first region holding the point (`lib.rs:757`), so the pointer is over exactly one region by
  construction (confirmed by the review against `Chrome::publish`'s push order). The list therefore
  holds one entry rising and at most one falling — two entries at most, of which exactly one is
  non-zero while the pointer is over a control. An entry that lands at zero is dropped, and an entry
  for a target that is not a control (the drag region, the panel's own surface) sits at 1.0 while it
  is under the pointer and is dropped the moment it is not. Nothing accumulates.
- `moving()` is `entries.iter().any(|e| progress(now - e.start, HOVER) < 1.0)`. It is false at rest
  and false once every transition has landed, which is what makes the host's frame clock terminate.
  `step` also drops an entry that has arrived at zero, so a pointer swept across ten controls does
  not leave ten entries behind.
- `instant` is `ChromeInput::reduce_motion`. A reduced-motion machine gets the value it is heading
  for on the frame the pointer arrives: hover is a question about where the pointer *is*, and a
  machine that has asked for less movement has asked for the answer rather than the animation.
  Implemented by starting the transition already past its span (`start = now - HOVER`), so there is
  one code path and one set of arithmetic.

The target, computed at the top of `Chrome::layout` and before `self.regions.clear()`, because it is
decided by the *previous* frame's regions:

```rust
let pointer = input.pointer.map_or(Hit::None, |(x, y)| self.hit(x, y));
let hit = match pointer {
    // The scrollbar has no single key of its own. `Hit::Scrollbar` is three-valued by
    // where down the track the point is (`lib.rs:547`), and its region is the 8px track
    // while the painter lights a 10px band (`overlays.rs:927-936`) — so a key taken
    // straight from `hit` would restart the width's transition on a vertical move and
    // would never start it in the outer two pixels. It is normalised to one key.
    Hit::Scrollbar(_) => Hit::Scrollbar(Scrollbar::Thumb),
    other => other,
};
let target = if hit == Hit::None {
    input.tabs.iter().find(|t| t.hovered).map_or(Hit::None, |t| Hit::Tab(t.index))
} else {
    hit
};
```

`TabInfo::hovered` stays the caller's way of saying which tab is hovered — it is what the host's own
`hit` call already answers (`host.rs:897`), and a test that names a hovered tab without inventing a
pointer still works.

Two more normalisations, both for the same reason — a key that a control cannot produce on its own:

- **The scrollbar's 2px band.** The painter's hover test is `x >= width - SCROLLBAR_HOVER`
  (`overlays.rs:927`), which is two pixels wider than the track it publishes. The target is therefore
  decided from the scrollbar's own geometry rather than from `hit`: while `self.scrollbar` holds a
  track (it is `None` when no bar is drawn, so there is nothing to hover), a pointer in the band and
  within the track's vertical span gives `Hit::Scrollbar(Scrollbar::Thumb)`.
- **The stepper's edge.** A stepper publishes two regions, `Less` and `More` (`overlays.rs:696-710`),
  so its *fill* is per-half and keyed per-half, but its *edge* is a property of the whole control
  (`overlays.rs:660`). The edge is driven by the larger of `of(Less)` and `of(More)`, which is the
  value of whichever half the pointer is in and zero when it is in neither.

### 2. The painter learns to fade

`Painter::fill` (`paint.rs:107`) becomes a call to a new `Painter::fill_at(rect, color, alpha)`, and
everything that was instant keeps its call. That is the whole change to the painter: no new blend
mode, no colour arithmetic.

**A draw at zero alpha is not a draw.** `premultiplied(ink, 0.0)` is `[0, 0, 0, 0]`, and
`stripped` (`tests.rs:2316`) returns a zero-alpha colour unchanged, so a fully transparent quad is
not one of the twelve palette colours and `the_chrome_only_ever_draws_the_chrome_palette`
(`tests.rs:2332`) fails on it — and the test that counts quads would see an extra one. So
`fill_at` returns early when `alpha <= 0.0`, and every place that draws a *second* quad for a
cross-fade guards on the same threshold the indicator's cross-fade already uses: `> 0.002`
(`strip.rs:511`). A control at rest therefore costs exactly the draws it costs today, and a control
mid-fade costs two.

`marks::draw` gains a sibling, `marks::draw_at(paint, mark, button, color, alpha, scale)`, which
does the same thing for the caption marks and returns early on a zero alpha. `draw` becomes
`draw_at(..., 1.0)`.

**Colours are never interpolated.** The palette test divides alpha out of every quad and asserts the
result is one of the twelve, so a quad holding a 50-50 mix of `ink_mid` and `ink` is not a colour
this crate may draw. Every transition here is therefore *two* draws at complementary alphas, which is
what the indicator's cross-fade already does (`strip.rs:511-532`) and what the marks' coverage ramp
already relies on (`paint.rs:129`). The one thing that moves continuously is geometry — the
scrollbar's width — and geometry is not a colour.

### 3. What each control animates

| Control | At rest | Under the pointer | What moves |
|---|---|---|---|
| Caption: close | `ink_mid` mark, no fill | `danger` fill, `ground` mark | the fill's alpha; the mark cross-fades |
| Caption: minimize, maximize | `ink_mid` mark, no fill | `ink` wash at 10% (`ink` at 0.10 alpha), `ink` mark | the wash's alpha; the mark cross-fades |
| New tab `+` | `ink_dim` text | `ink` text + an `ink` glow | the glow's alpha; the cross-fade |
| Settings mark | `ink_mid` | `ink` + an `ink` glow | the glow's alpha; the cross-fade |
| Tab | number `ink_dim`, no bar | number `ink_mid`, bar `signal_dim` | the bar's alpha; the cross-fade |
| Panel control | `ground` fill, `hairline` edge | `hairline` half-fill, `hairline-strong` edge | the half-fill's alpha; the edge cross-fades |
| Active tab | `ink`, weight 500 | `ink`, weight 500, no bar | nothing: the active tab outranks hover, as it does for the preview bar (`strip.rs:670-676`) |
| Rail item | no fill | `ground` fill | the fill's alpha |
| Menu row | no fill | `hairline` fill | the fill's alpha |
| Picker row | no fill | `hairline` fill | the fill's alpha |
| Scrollbar | 8px thumb | 10px thumb | the width, lerped by the hover value |

Three of these are new behaviour and not an animation of something that exists: the caption
backgrounds, the two glows, and the picker's row fill (which brings the picker into line with the
menu — both are lists of things to press, and DESIGN.md gives the menu that fill for the reason the
picker does not have it: "with no glyphs to read, the fill is what says what the pointer is about
to do", `DESIGN.md:240`).

The tab keeps its rule exactly: the number moves to `ink-mid` and the bar previews at `signal-dim`,
and **no background fill, ever** (`DESIGN.md:166`). The cross-fade is between the two inks the rule
already names, and it is suppressed on the active tab, whose number is `ink` at weight 500 whether
the pointer is on it or not (`DESIGN.md:162`) — a hovered active tab that dimmed to `ink-mid` would
be the strip's one rule losing to the pointer.

### 4. The caption backgrounds

Windows' own behaviour, because DESIGN.md is explicit that these are the most-hit pixels on the
machine and their muscle memory is not ours (`DESIGN.md:293`): the button fills, and the mark
inverts against the fill.

- Close at full hover is `danger` behind a `ground` mark. `ground` on `danger` measures 7.05:1 — the
  mark is dark on the coral, because `ink` on `danger` is 2.15:1 and unreadable. This is the one
  contrast pair this change introduces, and it goes in `palette.rs`'s floors table as a test.
- Minimize and maximize at full hover are `ink` at 10% over `surface`: a wash that says "this one"
  without spending a colour, at the strength Windows uses.
- The fill covers the whole 46x40 button, so the target a user aims at and the thing that lights are
  one rectangle.

### 5. The glow

A glow is a rectangle problem. The renderer draws a quad and a glyph and nothing else — no corner
radius, no blur, no gradient (`frame.rs:33`) — so the glow is a stack of flat rings: three
rectangles centred on the mark, each 2px larger than the last, `ink` at 12%, 6%, and 4%, drawn
outermost first. Over a `surface` that is a soft haze around the mark; the three alphas stack to
about 22% at the innermost ring.

- Rings are culled to the control's own rect, so a glow can never bleed into the tab beside it — a
  control that lights up its neighbour is worse than one with no hover at all.
- The glow is drawn under the mark, so the mark stays a one-pixel stroke on the pixel grid rather
  than a stroke over a bright patch.
- DESIGN.md says `hairline` is the only depth mechanism and there are no shadows. That sentence
  gains a carve-out and an argument: the glow is not elevation, it is the pointer's own footprint,
  and it is a flat stack of rectangles rather than a blur.

### 6. The host: a pointer move is a frame, and a transition is a clock

Two changes in `host.rs`, both required for any of the above to be visible.

**A pointer move that changes what is under the pointer asks for a frame.** In `moved`
(`host.rs:1381`), after `self.pointer` is set:

```rust
let at = self.chrome.hit(x as f32, y as f32);
if at != self.hover_at { self.hover_at = at; window.request_redraw(); }
```

`CursorLeft` (`host.rs:2217`) clears both and asks for a frame for the same reason. A move that
changes nothing asks for nothing, which matters: a terminal that repaints sixty times a second
because the pointer is crossing the grid is a terminal that has given up its idle cost.

**A transition in flight is a clock.** `about_to_wait` (`host.rs:2236`) gains one branch, before the
blink's: while `self.chrome.moving()`, the loop wakes in `HOVER_FRAME` (16 ms) and asks for a
redraw. The deadlines combine — the loop waits until the sooner of the next hover frame and the next
blink, and parks in `Wait` when neither is pending, which is where an idle terminal spends its life.
The existing blink logic is not restructured, only given a second deadline to be the minimum of.

### 7. DESIGN.md

Three edits, all of them narrowing a claim that this change widens:

- **Motion** (`DESIGN.md:317`): the section says "Everything else is instant" (`DESIGN.md:333`) and
  "No other transition in the app is load-bearing, so no other transition exists" (`DESIGN.md:350`).
  Hover transitions are not load-bearing and do exist, so both sentences become a rule with two
  tiers: a transition that carries an answer (the two that are there today, 140 ms and 180 ms) and
  the hover fade (110 ms, and it carries only "the pointer is here"). The fade earns its place by
  what a moving pointer looks like without it: flash as the pointer crosses a row of tabs, and the
  transition is what makes the strip read as continuous.
- **Window chrome** (`DESIGN.md:288`) and **Tabs**: the caption hover becomes a fill, and the `+` and
  the settings mark get the glow.
- **Palette rules** (`DESIGN.md:54`): "no shadows, no gradients" keeps its force and gains the glow
  as the named exception, with the reason: it is a stack of flat rectangles, and its job is to show
  where the pointer is rather than to lift a surface off the page.

## The tests

The crate's tests build a `Chrome`, hand it an input, and read colours back out of a frame. Hover is
now a function of the clock, so every test that hovers has to say *when*.

New, in `crates/zet-ui/src/tests.rs`:

1. `a_caption_button_fills_under_the_pointer_and_only_that_one` — with the pointer on minimize, one
   wash appears, sized to the button, and the other two buttons gain nothing.
2. `the_close_button_fills_danger_and_its_mark_turns_dark` — the fill is `danger` at full hover and
   the mark under it is `ground`, at the same place the mark was.
3. `a_hover_fades_in_over_the_time_the_document_gives_it` — at 0 ms the fill is absent, at half the
   span it is there at partial alpha, and at the span it is at full. Asserts the curve, not just the
   endpoints.
4. `a_hover_that_leaves_fades_back_out` — the pointer moves away and the fill does not vanish on the
   frame it moves; the value falls and lands at nothing.
5. `the_new_tab_mark_and_the_settings_mark_glow_under_the_pointer` — each gains `ink` rings that are
   larger than its mark, inside its own rect, and at no more than the glow's alpha; neither has any
   when the pointer is elsewhere.
6. `a_glow_never_reaches_the_tab_beside_it` — the rings are contained in the control's own rect.
7. `reduce_motion_lights_a_control_on_the_frame_the_pointer_arrives` — value 1 immediately, and
   `moving()` false on that frame.
8. `nothing_is_moving_once_every_hover_has_landed` — `moving()` true mid-fade and false after, which
   is the property the host's frame clock terminates on.
9. `a_menu_row_and_a_picker_row_fill_as_the_pointer_crosses_them` — both lists get the fill, and it
   is the same fill.
10. `the_scrollbar_grows_over_the_time_the_hover_takes` — the thumb's width is 8, then between 8 and
    10, then 10.
11. `every_control_lit_at_once_still_draws_the_palette` — the existing palette test, run with the
    pointer over a caption and over the panel, so the wash and the rings are covered.

Changed: the two existing tests that put the pointer on a control and then expect the lit state on
the first frame — `a_hovered_tab_gains_a_bar_and_nothing_else` (`tests.rs:795`, which sets
`TabInfo::hovered`) and `the_hover_fill_covers_the_region_a_click_would_take` (`tests.rs:1170`,
which passes a pointer). Each gets a `set_time` pair: settle at rest, move the pointer, advance past
the span. That is the honest fix rather than a special case in the code: an animated hover that is
already lit on the frame the pointer arrives is not an animation. No other test in the crate hovers
anything — `open_menu_at` builds its input with `pointer: None` (`tests.rs:3254-3261`), so the menu
row fill is only reached by the new test above.

## What is deliberately not here

- **No hover on the drag region, the tab strip's own surface, the panel's surface, the menu's
  padding, or the find bar's field.** They are not controls; a hover on a surface that does nothing
  is a hover that lies.
- **No press state, no ripple, no scale.** Motion is a budget and the request was hover.
- **No animation of anything that is not a pointer arriving or leaving**: tab open and close stay
  instant, and the panel still closes on the frame it closes on, for the reasons DESIGN.md already
  gives (`DESIGN.md:335`).
- **No new palette entry.** Everything fades a colour that is already in the palette, which is what
  keeps the palette test — and the two-plane rule it exists for — untouched.

## Files

- `crates/zet-ui/src/hover.rs` — new: the lit values and their transitions.
- `crates/zet-ui/src/lib.rs` — the target, the step, `Chrome::moving`, and the plumbing.
- `crates/zet-ui/src/strip.rs` — captions, the `+`, the settings mark, the tab cross-fade and the
  preview bar, `HOVER`.
- `crates/zet-ui/src/overlays.rs` — the panel's controls and rail, the menu, the picker, the
  scrollbar.
- `crates/zet-ui/src/paint.rs` — `fill_at`.
- `crates/zet-ui/src/geometry.rs` — `HOVER_MS`, `HOVER`, the glow's constants, `HOVER_FRAME`.
- `crates/zet-ui/src/marks.rs` — the rings.
- `crates/zet-config/src/palette.rs` — one more contrast floor.
- `crates/zet/src/host.rs` — the pointer-move redraw, the hover frame clock.
- `crates/zet-ui/src/tests.rs` — the tests above.
- `DESIGN.md` — the three edits above.

## What the review changed

A subagent read this plan against the code and found four things worth acting on, all now folded in
above.

1. **Everything at rest would have failed the palette test.** A quad at alpha zero is `[0,0,0,0]`
   after premultiplication, `stripped` hands that back unchanged because it divides by an alpha of
   zero, and zero is not one of the twelve colours — so `the_chrome_only_ever_draws_the_chrome_palette`
   would have failed on the first frame, and the quad-counting tests would have seen a phantom quad.
   The guard is now in the painter (`fill_at` returns on a zero alpha) and in every complementary
   draw, which is the same `> 0.002` the indicator's cross-fade has always used.
2. **The scrollbar's hit region is not what the painter lights.** `Hit::Scrollbar` is three-valued by
   where down the track the point is, and the region published is the 8px track while the painter
   lights a 10px band — so a key taken straight from `hit` would restart the width's transition on a
   vertical move and never start it in the outer two pixels. Normalised to one key, decided from the
   bar's own geometry.
3. **A stepper is one control with two keys and no key for its edge.** The edge now takes the larger
   of the two halves' values.
4. **A hovered active tab would have dimmed.** The strip's rule is that the active tab is `ink` at
   weight 500, and hover moves a number to `ink-mid`; the preview bar already excludes the active
   tab (`strip.rs:670`). The text now does too.

Left as it is, with reasons: `Hit` keys are indices that move on a discrete event — a tab closing
renumbers every tab behind it, and a diagnostic appearing shifts every line after it — so a
transition in flight restarts when one of those events happens. It is a 110ms fade over a frame in
which the user has just done something that changed the list, and a stable identity for a row would
mean the chrome holding a copy of the caller's list. Not worth it.

Also corrected: eight line citations, one of which (`host.rs:890` for the tab hover) pointed at the
wrong function.

## What was built

All of it, and the plan above is accurate about the shape: one `Hover` keyed by `Hit`, stepped
from the target the layout computes, read as a number by every painter that has a hover. Twelve new
tests in `zet-ui` (110 in the crate), one new contrast floor in `zet-config`, and the whole
workspace is 1047 tests green with `cargo clippy --workspace --all-targets` and `cargo fmt --check`
clean.

The tests, as they ended up named:

1. `a_caption_button_fills_under_the_pointer_and_only_that_one`
2. `the_close_button_fills_danger_and_takes_a_mark_that_is_readable_on_it`
3. `a_hover_fades_in_over_the_time_the_document_gives_it` — sampled at an eighth, a half and the
   whole of the span, so the curve is asserted and not just its ends.
4. `a_hover_fades_back_out_when_the_pointer_leaves` — including that the fill is still at full on
   the frame the pointer leaves, which is what a transition turned around from where it is means.
5. `the_new_tab_mark_and_the_settings_mark_glow_under_the_pointer` — the rings are sorted by area
   and asserted to fall off outward, because a stack of equal alphas is a flat rectangle.
6. `a_glow_stays_inside_the_control_that_owns_it`
7. `reduce_motion_lights_a_control_on_the_frame_the_pointer_arrives`
8. `the_chrome_says_it_is_moving_until_every_hover_has_landed`
9. `a_menu_row_and_a_picker_row_fill_under_the_pointer`
10. `the_scrollbar_grows_over_the_time_the_hover_takes`
11. `a_hovered_active_tab_keeps_its_ink_and_gains_no_bar` — the review's fourth finding, as a test.
12. `every_hover_is_a_palette_colour_at_a_coverage` — the palette rule with a partial alpha live in
    the frame, which is the frame it is easiest to break in.

Where the build deviated from the plan:

- **`HOVER_FRAME` lives in `host.rs`, not `geometry.rs`.** It is not a number the chrome's layout
  has any use for: it is how often the *window* asks for a frame, which is a decision about the
  event loop and nothing else.
- **The glow's constants live in `marks.rs` and `strip.rs`, not `geometry.rs`.** `GLOW_RINGS`,
  `GLOW_STEP` and `GLOW_ALPHA` are the rings' arithmetic and `PLUS_GLOW` is the size of one mark's
  box; neither is layout the strip and the panel have to agree about.
- **`CAPTION_WASH` and `PLUS_GLOW` are `pub(crate)`.** A fade is only assertable at its ends and
  partway along if a test can name the value it is heading for, which is the same reason
  `marks::MARK_BOX` already was.
- **`Chrome::hover_target` is called from `layout`, not `plan`.** `layout` clears the region list
  before it calls `plan`, so a target read from the regions has to be taken before the clear. It is
  the same reason the method exists at all: the regions are what "under the pointer" is decided
  from, and they are the previous frame's.
- **`marks::draw` takes an alpha rather than there being a second `draw_at`.** The first version had
  a wrapper for the four callers that wanted a full-strength mark, which is a name for `1.0`.
- **Two `#[allow(clippy::too_many_arguments)]`**, on `Chrome::overdraw` and `marks::sliders`: the
  hover is an eighth argument to both, and neither has a group of arguments that wants to be a
  struct. `overlays::draw_control` already carried the same allow for the same reason.
- **One more `DESIGN.md` edit than the three planned**: the tab bullet now says a hovered *active*
  tab is untouched, because that rule is the one the review found and the one a later reader is
  most likely to break by "fixing" the suppression.
- **`settled()` draws until the chrome stops moving, not twice.** Two frames was the first
  implementation and it was wrong: the entry is created on the frame *after* the region is
  published, so a two-frame draw lands on a value of zero. It advances a whole span per frame
  because past the span a transition is at the value it was heading for.

## Superseded (2026-09-27, later the same day)

The two marks' glow is gone. The new-tab mark and the settings mark no longer draw three rings of
`ink` at 12%, 6% and 4% around themselves; what the pointer does now is turn the mark's own ink
white — `ink-dim` to `ink` for the `+`, `ink-mid` to `ink` for the settings bars — over the same
110ms fade.

Two reasons, both about what a ring stack is at this size. It is three flat rectangles drawn beside
a mark that is otherwise nothing but rectangles, which at the size of a strip control reads as a
fill behind the mark rather than as a light on it. And a hover that adds a shape changes the
control's footprint in the frame, where every other hover in the chrome is a coverage or a width
moving on a control that was already there.

`marks::glow`, `GLOW_RINGS`, `GLOW_STEP`, `GLOW_ALPHA` and `strip::PLUS_GLOW` are deleted with it,
and `a_glow_stays_inside_the_control_that_owns_it` is now
`a_hovered_mark_reaches_no_further_than_the_control_that_owns_it`, which asserts the same thing
about ink that does not leave its cell. Everything else in this document stands: the fade's
`progress`, the 110ms span, the reduce-motion rule, the clock living in the window, and `Hover`
keyed by `Hit`.

The settings mark also gained a state this document did not have: it is `ink` for as long as the
settings tab exists, pointer or no pointer, because what is open is what is bright.
