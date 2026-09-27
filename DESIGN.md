# zet design system

## The world: Instrument

zet looks like a precision instrument, not like a terminal.

The reference points are measuring equipment and studio hardware: an oscilloscope, a
VU meter, a rack tuner, a laboratory power supply. What those objects share is that
every mark on them carries information, nothing is decoration, and the numbers are
the interface. They are dense without being cramped, quiet without being dull, and
they use exactly one signal color to say "this one is active."

That last property is why this world fits zet. A terminal multiplexer's whole job is
to say which of several sessions you are in. An instrument answers that question
with a single lamp, not with a highlighted card.

### Two planes, never mixed

The window has two planes and they never borrow from each other.

**The chrome plane** is zet's: the titlebar, the tab strip, the settings page, the
scrollbar, the find bar, every focus ring. It uses the zet palette below, including
the signal color.

**The grid plane** is the program's: every character cell, its foreground, its
background, the selection, the cursor. It uses only the active theme's ANSI palette.
zet never tints a cell to match the app.

This rule is what keeps zet from looking like an app that painted over your terminal.
The signal color appears in the chrome and stops at the grid boundary.

## Color

### Chrome palette

| Token | Value | Use |
|---|---|---|
| `ground` | `#0a0b0d` | Window background, behind everything |
| `surface` | `#0f1114` | Titlebar and tab strip |
| `surface-raised` | `#15181c` | Settings page, popovers, menus |
| `hairline` | `#23272d` | Every 1px division in the chrome |
| `hairline-strong` | `#333941` | Focused input borders, drag targets |
| `ink` | `#e7e9ec` | Primary chrome text, active tab index |
| `ink-mid` | `#99a0a8` | Secondary text, hovered tab index |
| `ink-dim` | `#7b838d` | Inactive tab index, disabled controls |
| `signal` | `#ffa62b` | The active marker. Indicators only |
| `signal-dim` | `#8a5a17` | Signal at rest, for hover preview of an indicator |
| `danger` | `#ff6b5e` | Destructive confirmations, error text |
| `ok` | `#57d9a3` | Success confirmations |

Rules that keep the palette honest:

- `signal` never fills an area larger than 3px by 40px. It is a lamp, not a color.
- `hairline` is the only depth mechanism in the chrome. No shadows, no gradients, no
  glass. Elevation is expressed by a line and by a one-step surface change.
- The only transparency in the chrome is a coverage on a palette colour. A hover fades a
  colour by lowering its alpha, never by mixing it with another, and a mark lit under the
  pointer is that mark's own ink at a higher value rather than a second shape around it,
  because the renderer draws rectangles and glyphs and nothing else. A quad at zero alpha is
  not a colour at all, so a fade that has reached nothing draws nothing.
- Text on `surface-raised` is still `ink` or `ink-mid`, never a tinted gray.
- The settings page shows every value numerically, so it is legible as data even when
  the visual preview is on another tab.

### Contrast floors

| Pair | Minimum |
|---|---|
| `ink` on `ground` | 12:1 |
| `ink-mid` on `ground` | 6:1 |
| `ink-dim` on `surface` | 4.5:1 |
| `signal` on `surface` | 4.5:1 |
| `ground` on `danger` | 4.5:1 |
| Theme ANSI colors on theme ground | 4.5:1 for zet's own themes |

Imported palettes ship unmodified so they look like themselves, and the theme picker
labels them "as published" rather than implying they were checked.

These floors are asserted in tests, and that has already changed one value: `ink-dim`
was first drafted as `#5a626b`, which measures 3.06:1 on `surface` and fails the row
directly above it. The floor is the requirement and the hex was a guess, so the hex
moved to `#7b838d` — the same grey, lighter. A palette edit that dims the chrome text
should fail a test rather than a review.

Two theme colours are exempt from the ANSI floor, and the exemption is deliberate:
indices 0 and 8 are a palette's "black" and "bright black", which programs use as
backgrounds. Requiring them to be legible against a near-black ground would force them
to be light, at which point they stop being black and the programs that rely on that
stop looking right. The other fourteen are colours text is printed in, and they all
clear 4.5:1.

## Type

Two faces, and they do not swap roles.

**Chrome: IBM Plex Sans.** Open source, self-hosted, and drawn with engineering
proportions rather than marketing ones. Two weights ship, 400 and 500. Nothing in
the chrome is bold; 500 is the ceiling and it marks only the active tab and the
settings section headings.

**Grid: Cascadia Mono by default**, discovered from the system with Consolas as the
fallback. The terminal font is a user setting with a curated list plus any installed
monospace face.

Numbers in the chrome use tabular figures. Tab indices, font sizes, opacity
percentages, scroll positions, and row counts all align on the same column width,
which is the single detail that makes the chrome read as instrumentation rather
than as an app.

This costs nothing to hold up, because the face already does it: Plex Sans's figures
are tabular by default — every digit at both weights advances 600/1000 of an em — and
the font carries no `pnum` feature to switch away from them. There is no OpenType
feature for zet to ask for and no spacing to fake by hand. It is a property of a file
rather than of the code, so `zet-ui`'s `fonts` module has a test that rasterises all
ten digits from the shipped `.ttf` and fails if they ever stop agreeing.

Scale, at 100% DPI:

| Role | Size | Weight | Tracking |
|---|---|---|---|
| Titlebar app name | 12px | 500 | +0.02em |
| Tab index | 13px | 500 active / 400 inactive | 0 |
| Settings section heading | 12px | 500, sentence case | +0.08em |
| Settings label | 13px | 400 | 0 |
| Settings value | 13px | 400, tabular | 0 |
| Body text | 13px | 400 | 0 |
| Hint and help | 12px | 400 | 0 |

The chrome does not scale with the terminal font size. Text scaling is a separate
setting and follows the Windows accessibility text size value by default.

## The tab strip

This is the part of the brief that has to be exactly right.

A tab is a number and a name. The number says which channel; the name says what is
running on it. `#3  PowerShell` is one cell, and the number is the part that never
changes: it is the tab's identity, and the name is what the program on it is doing
right now.

```
 horizontal, one row shared with the titlebar
+---------------------------------------------------------------+
| zet   #1 pwsh   #2 vim   #3 dotfiles  (drag)   _   []   x     |
|       ~~~~                                                    |
+---------------------------------------------------------------+
```

- The index renders as `#` at 70% of the number's size, then the number. The
  `#` is part of the mark, not a prefix that disappears. It is what makes the
  strip read as a set of numbered channels.
- Numbering is by position, and closes up behind a tab that goes. Close #2 of three
  and the remaining tabs are #1 and #2, and the next tab opened is #3. A number is
  where a tab sits and nothing else, because the strip is the one place it is read
  and a gap there is indistinguishable from a tab that failed to open. The tab on
  screen keeps the mark across a close: it moves down a number with everything else
  rather than the user being handed a different terminal.
- Single digit up to #9, then `#10` and beyond. Tab width grows to fit.
- The name is what the program set with `OSC 0`/`OSC 2`, or the profile's name
  when it set nothing. It is one `ink` step quieter than the number in colour
  and one weight lighter, so the row still scans by number.
- A name too long for its cell is cut and given an ellipsis. A name that fits
  whole is never cut to make room for one.
- Tab width is capped at 180px. Past the point where the names fit, every cell
  gives up the same amount, down to a floor of the number alone — so a crowded
  strip degrades evenly instead of being one wide tab and a row of clipped ones.
  Below the floor the run overflows and the tabs past the end are not drawn.
- Active tab: a 2px `signal` bar on the bottom edge plus `ink` at weight 500.
- Inactive tab: `ink-dim` for the number and `ink-mid` for the name, weight 400,
  no bar, no background. The name is brighter than the number at rest: the
  number is an ordinal, and the name is the thing being read.
- Hover: number moves to `ink-mid`. The indicator bar previews at `signal-dim`.
  No background fill, ever. Filling a hovered tab is the single most common way a
  tab strip starts looking like everyone else's. A hovered *active* tab is
  untouched: its index is `ink` and its bar is already there, and the tab that is
  open outranks the pointer.
- New tab: a `+` at the end of the run, the footprint of a tab with no name,
  `ink-dim`, standing one `CONTROLS_GAP` of 12px clear of the last cell. A mark butted
  against the run reads as the next tab in it, which is exactly what it is not: the run
  has to say where it stops before the control does.
- Every cell carries a × when the pointer is on it, in the `CLOSE_BAND` of 18px the
  cell reserves at its right. A name is fitted to end at that band, so the mark, the
  gap in front of it and the gap behind it are all decided before the pointer arrives
  and nothing reflows when it does — and the mark is 8px rather than the caption
  buttons' 10, because a control inside a cell is not a button with a face of its own
  and at ten it was the heaviest thing in a strip whose numbers are set at 13. The band
  is wider than `TAB_PADDING` for the same reason the mark is smaller: two names sit
  `TAB_PADDING` apart and read as two cells, so a mark the same distance from a name
  reads as part of it. It is drawn only while the pointer is on the cell, so a strip
  with no pointer on it is the strip it always was, and the × cannot be reached for
  without the cell it belongs to lighting first. Its ink is `ink-mid` anywhere in the
  cell and `danger` inside the mark itself — the caption's close button spends the same
  colour for the same reason. That last step is the one hover in the chrome that is not
  a fade: the mark does not exist until the pointer is already inside its own cell, so
  the two are one control seen at two distances rather than two controls the pointer
  crosses between. The settings cell has one too: it is a tab like the others, and a tab
  that cannot be closed the way it is closed is a tab that teaches the wrong gesture.
- The OS window title is the active tab's name and then `zet`, so the taskbar and
  Alt-Tab distinguish two zet windows. The drawn strip keeps `zet` in its own
  slot: the names are already on the tabs.

```
 vertical, a rail on the left
+------+--------------------------------------------------------+
| zet  |                                    _   []   x         |
+------+--------------------------------------------------------+
| #1 | |                                                        |
| #2   |   PS D:\Apps\projects\zet> cargo build                 |
| #3   |      Compiling zet-vt v0.1.0                           |
|      |       Finished dev in 0.34s                            |
|  +   |                                                        |
+------+--------------------------------------------------------+
```

- Rail width 48px, cells 36px tall, index centred in what the × leaves — the rail
  reserves the same `CLOSE_BAND` a horizontal cell does, so both positions carry the mark
  in the same place. It is the one position where a tab's number is not centred in its
  cell, and that is the price of the strip behaving the same way in both.
- The rail carries numbers and no names. It is 48px wide and the whole reason to
  choose it is that it gives the grid the rest, so a name in it would be a name in
  the space the tabs were moved aside to free. This is the one position where a tab
  is only a number.
- The divider between the rail and the grid is a `hairline`.
- Active tab carries the same 2px `signal` bar, moved to the left edge. Same
  language, rotated.
- The rail has no scrollbar. Overflowing tabs compress the cell height to a 24px
  floor, then the rail scrolls under the wheel.

### Zero tabs

When no terminal is open the strip does not collapse to a thin line and does not
show an empty state message. It is simply absent. The window is a titlebar, a
hairline, and the grid with the whole height back.

The whole row that held tabs is gone. The window is 40px shorter. This is the
brief's requirement and it should feel like the app got lighter, not like something
is missing.

A centered line naming the two chords was the plan, and it is not there, because the
state it would appear in is unreachable. A window with no terminals left closes, and the
one thing that used to be able to hold it open — a settings page with nothing behind it —
is a page rather than an empty strip: the page is the whole content area, so what the
user sees is the settings and not a strip with a gap in it. The window that closes does
so without drawing the strip it no longer needs, which is a fact about the host rather
than a hope about the order of two calls: the redraw returns before it plans a frame once
the loop has been told to stop. An empty state for a state nobody can reach is a line
nobody reads.

### The tab menu

A right-click on a tab opens a menu of what can be done to it. It is the mouse's way
to the chords, and it is the only menu in the window: a terminal is a place where the
program owns most of the pixels, and the strip is the part that is ours.

```
                              +---------------------+
                              |  New tab            |
                              |  New window         |
                              |  Close tab          |
                              |  Close other tabs   |
                              +---------------------+
```

- Four items, in that order: new tab, new window, close tab, close other tabs. `Close
  other tabs` is offered only when the window holds more than one tab. An item that
  would do nothing is an item that teaches the user the menu does not work, which is
  the same reason the thickness row appears only beside a shape that has one.
- Closing is last of the three that are always there, because it is the one that takes
  something away and a menu whose destructive item sits in the middle is a menu where a
  mis-aimed click lands on it.
- The menu is `surface-raised`, a `hairline` border, 28px rows, 16px of padding above,
  below, and at each side, and 120px of minimum width. It is the settings page's own
  surface and its own border, one level smaller — a menu is that language shrunk to the
  size of the question it is asking.
- Hover fills the row in `hairline`, the same fill the page gives the half of a control
  a click will take and for the same reason: with no glyphs to read, the fill is what
  says what the pointer is about to do.
- Width comes from measuring the items, not from a constant, so renaming one moves the
  rectangle and the floor is only there for a menu of one short word.
- Placement: it opens with its top-left corner at the pointer and **flips** rather than
  clamps. Too near the right edge it moves left until it fits; too near the bottom it
  opens upward. Clamping would pin it to the edge with the pointer somewhere in the
  middle of it, which for a menu means the item under the pointer is not the one that
  was aimed at. A menu wider or taller than the window is pinned to the corner, since
  there is no side left to flip to.
- It is drawn over everything, caption buttons included, and it takes the click:
  a press inside it is answered by the menu and never by what it covers. It covers the
  caption buttons only when it was opened under them.
- "Over everything" is a fact about the frame rather than a claim about the order of five
  calls. The chrome is drawn as pairs of batches — every rectangle in one, every glyph in
  the next — and that rule is what puts a title's text over a control's surface. The menu
  is the one thing that cannot live under it, so it is drawn as a pair of its own, after
  all of them: its surface over the chrome's text, and its own labels over its own surface.
  A menu whose surface shares the chrome's rectangle batch is a menu with a tab's title
  legible through it, which is what it was before this.
- The next press anywhere else — on another tab, on the terminal, on a caption button —
  is what dismisses it. No timeout, no click-to-toggle, and no fade *out*: the fade is
  the menu's arrival, over the same 120ms as the other two surfaces drawn over the
  window, and a dismissal is a cut. A menu that faded on its way out would be a menu
  where the press during the fade lands on the window underneath.
- `Escape` closes it and is the menu's key for as long as it is open. It is the one
  key the menu takes: everything else still goes to the shell, for the reason a page in
  front of it gives none of them.
- The menu belongs to the tab it was opened on, and it goes away when that tab does —
  including when a tab is closed from somewhere else, and when the program on it exits.
  A menu still offering `Close tab` for a tab that is gone is a menu that lies.

## Cursor

The cursor is the one element that belongs to both planes, so it follows the theme's
palette and never the chrome's.

- Default: solid block, drawn in the theme foreground with the cell's character
  inverted to the theme background.
- Options: block, bar, underline, hollow block. Thickness is adjustable from 1 to 8px
  for bar and underline, which is the accessibility lever for low vision.
- Blink: 530ms on, 530ms off, hard transitions, no fade. Blinking curses if you can
  see the in-between. Off is a first-class setting and is the default when
  reduce-motion is on.
- Unfocused window: hollow block outline, no blink. Never invisible.

## Window chrome

One 40px row holds the app name, the tabs, the new-tab mark, the settings control, the
drag region, and the caption buttons. Stacking a titlebar above a tab strip wastes 36px of
vertical space to say nothing, and this is the change that makes zet feel denser than
Windows Terminal.

- Caption buttons keep Windows' own metrics: 46px wide, 40px tall, full-height right
  edge. Under the pointer the button fills: a tenth of `ink` for minimize and maximize,
  and `danger` for close, whose mark turns over to `ground` so that it stays readable on
  the coral — `ground` on `danger` is 7.05:1, where `ink` on it is 2.15:1. Users hit these
  hundreds of times a day and muscle memory is not ours to redesign.
- Two controls sit after the last tab: `+` for a new tab, and the settings mark that opens
  the settings tab. `+` is one number cell wide, because it stands where a tab would; the
  settings mark is a fixed 16px, because it is not a tab and has no number to make room
  for. The two are adjacent — nothing between them — and the pair stands 12px clear of the
  run, so the pair's centres are 28.5px apart, where a cell-wide settings control would
  have put them 41px apart. That gap is the run's own end rather than the pair's spacing:
  the two marks are one cluster and the tabs are not part of it. Both are
  drawn only when they fit — a control is never the thing that overflows, and the tabs
  stop before them rather than running under them. The settings mark goes first: 16px is
  the whole difference between a row that holds both and a row that holds one, and the
  chord that opens the tab is still there.
- The settings mark is geometry, like the caption marks and for the same reason: three
  bars with a tick on each. A gear would be a ring and eight teeth, which is a polygon
  rasteriser in a module whose whole subject is a one-pixel stroke that lands on exactly
  one pixel.
- Both of those controls are bare marks with no face of their own, and neither gets one
  under the pointer: what the hover does is turn the mark's own ink white — `ink-dim` to
  `ink` for the `+`, `ink-mid` to `ink` for the settings bars. No halo, no bloom, and
  nothing drawn beside the mark, so a hover here is a colour and not a second thing. The
  settings mark is `ink` for as long as the settings tab exists, pointer or no pointer,
  because the strip's rule that what is open is what is bright applies to the view as well
  as to the tab: under the pointer a lit mark has no second look to give.
- The settings tab itself is a cell at the end of the run of tabs, titled `settings`, with
  no number on it. It is a view of the app rather than a program in a shell, so numbering
  it would either break the run from `#1` or leave a gap where a shell used to be. The mark
  and the cell answer differently on purpose: the cell is the tab, the mark is the button
  that opens it and closes it, and a press has to land on the one the user pointed at. The
  cell carries a × like every other cell, and the mark is not a substitute for it.
- The drag region is any horizontal gap between the last of those controls and the
  caption buttons. Double-click maximizes, and restores a window that is already
  maximized.
- Maximized: the row loses its bottom hairline and the window loses its rounded
  corners, matching how Windows handles a maximized frame. The rounding itself has to
  be asked of the compositor rather than inherited from it: a frameless window has
  overridden its own non-client area, and Windows leaves those square in both states.
  So a restored zet asks for round corners explicitly and a maximized one asks for
  square ones, which is the only way the design's promise holds on a machine whose
  default would have been square anyway.
- The app name is hidden when the tab strip needs the space. Tabs outrank branding.

## Motion

Three authored moments, and each of them carries an answer that the frame it lands on
cannot.

**The active indicator travels.** When the active tab changes, the 2px bar moves
from the old tab to the new one over 140ms on an exponential ease-out curve, and the
two indices cross-fade their ink weight over the same duration. The strip itself
does not move, resize, or reflow. One line travels; nothing else reacts.

**A hover lights.** Over 110ms, on the same curve, from the control the pointer left to
the one it has reached. Every hover in the app is this one transition: a caption button
fills, a tab's index moves to `ink-mid` and previews its bar, the new-tab mark and the
settings bars turn white, a menu or picker row fills, a settings rail item and the whole
face of a control fill, and the scrollbar grows from 8px to 10px. Nothing about the layout
changes — a hover moves a coverage, and one width — so the frame it lands on and the
frame it starts on are the same frame with different numbers in it. What the fade buys
is that a pointer crossing a row of tabs is one movement rather than a row of hard cuts.

**A surface appears.** Over 120ms, on the same curve, from nothing to everything. The
three surfaces in the chrome that are drawn *over* what is on screen — the tab menu, the
picker, the find bar — and one thing besides: a tab that has just arrived. All four are
the same question, "how much of this is there", and they are one piece of arithmetic with
a key apiece. A menu that appears over a terminal, a find bar that appears over the grid:
what the fade buys is that the arrangement underneath stays legible through the arrival,
so a press on something is a press on a thing that was already there rather than on a
thing that appeared between the press and the release.

**A departure is a cut.** A surface that goes is gone on the frame it goes. What fades in
is the frame's worth of ink that a dismissal would otherwise take with it, and a
dismissal is the user's own act — the pointer has already moved to what is revealed, and
holding the dismissed surface over it for another 120ms is the interface disagreeing with
its own hand.

So the rule, and it is the whole of it: *a surface that appears over what is on screen
fades in; a surface that replaces what is on screen is there on the frame it opens.* The
settings page is the second kind. It is the grid's own rectangle with the grid not drawn
in it, so there is nothing to fade *against* — it would be a crossfade between a thing
and the absence of that thing, which is a lie about what a tab is. A tab is a change of
what is on screen, not a change of place, and a page that animated into existence would
delay the controls inside it by exactly as long as the animation.

Everything else is instant:

- Theme changes apply in one frame with no crossfade. A crossfade would show an
  intermediate theme, which is a worse lie than a hard cut.
- The settings tab opening and closing. The tab replaced a panel that slid in from the
  right edge, and the slide went with it: it opens and closes by replacing the grid, and
  what replaces is there on the frame it opens.
- Tab open and close: the new tab appears already at full size. No scale-in, no
  slide. A tab that animated into existence delays input by exactly as long as the
  animation, which in a terminal is a real cost. What a new tab does get is the ink of
  everything drawn in it, over the same 120ms as a surface — the tab's own width is
  settled from its first frame, so the strip never reflows underneath it.
- Nothing that moves the layout is animated, and the grid is never animated. The strip
  and the page are divided by the same arithmetic on every frame, and a terminal that
  eased into its new size would be a terminal printing into a rectangle it does not own.
- Reduce motion, read live from the system: the indicator jumps, a hover is lit on the
  frame the pointer arrives, a surface is at full on the frame it opens, cursor blink
  stops. Everything still works, nothing moves.

The bar travels because that motion carries the answer to "where am I now" — an answer
about space. The hover fades because a pointer arriving and a pointer leaving are worth
telling apart — an instant hover is a flicker at the speed a mouse moves — and that is an
answer about attention. A surface arrives by rising because a menu that appears in one
frame is a menu the user reads before they have decided to look at it, and because the
thing under it was there first. No other transition in the app is load-bearing, so no
other transition exists.

## Settings

The settings are a tab: the content area, on `surface-raised`, from the bottom of the strip
to the window's bottom edge and from the window's left edge to its right. Everything below
the strip, and nothing else — the strip stays visible and stays clickable, because the
settings tab is one of the things in it.

A tab rather than a panel, and the difference is the geometry rather than the wording. A
panel is an overlay: it covers part of a terminal that is still drawn behind it, it has an
edge to be separated from that terminal, and there is a screen underneath to be leaked
into. A tab is what is on screen instead of the terminal, so the grid draws nothing at all
while the page is up and there is no sliver of shell beside, above or under it — which is
the whole reason it is a tab. The page's own rectangle is the grid's rectangle, the same
one the terminal would have had, which is why the find bar takes height off both.

The tab appears in the strip at the end of the run of tabs, titled `settings`, and has no
number on it: it is a view of the app rather than a program in a shell, and `#3` beside two
shells would say otherwise. It is opened and closed by the settings mark in the strip and
by a chord, and both go through the same toggle, because a button and a binding that
disagreed would be one of them wrong. The tab stays open while a terminal is in front of
it: it is still in the strip, the mark stays lit, and a press on its cell brings it back
where it was left. It is closed by the mark, by `Escape`, by the close-tab chord, by the ×
on its cell while the pointer is there, and by a middle-click on the cell — five doors
onto one function, because a control and a binding that disagreed would be one of them
wrong. Closing it leaves the shell behind it exactly as it was: it is the page that goes,
not the terminal the page was covering.

The page is why the window has an ending to state. When every tab was a shell, "the last
tab closed" and "there is nothing left" were the same sentence, and closing the last one
quit. They are two sentences now, so the window asks the second: it closes when there is
no terminal and no page, and not before. A shell exiting under a page puts the page in
front of the window rather than taking the window with it, because a page that was behind
a terminal that is gone is a page the user cannot see on a window still in front of them.
The window that does close accepts no more frames — it is not an empty window on its way
out, it is the loop stopping, and the strip's empty state stays a state nobody reaches.

Quitting is a different question and has a different answer. The close button, the
system's close, and `Alt-F4` end the window whatever is in it: the user has said what
they want, and weighing that against the tab count would be the interface arguing with
them. Only the tab-level doors — the ×, the chord, the middle click, the menu's `Close
tab` — consult what is left.

The terminal is not visible behind it, because there is no behind: the settings tab and a
shell are two tabs, and the shell is a tab that stopped being the one on screen. It is
still running, and it was never resized or closed by opening the page. What the page has
instead of a preview is a promise: every change writes through immediately. Theme, font,
size, opacity, and background all apply the moment they change, and switching back to the
shell shows them already applied. That is the reason the page exists at all rather than a
config file alone.

The page is a rail and a column: 180 of sections down the left, then the rows. The rail is
what this grew by; the page beside it takes the rest of the width, so a wide window gives
the rows the room rather than leaving a 560px stripe of settings on a 2400px screen.

Sections, in order: Appearance, Tabs, Terminal, Keys — plus Problems, which exists only
while the configuration file has something wrong with it and is why a section is named
rather than numbered. The rail lists them in the page's own type and in the case they are
spelled in, with the one being shown filled in `hairline` and the one under the pointer in
`ground`: the same pair of fills a control and its hover use, because the rail is part of
the surface rather than a second thing bolted to its edge. Nothing in the page shouts: a
heading is a heading because of its size, its tracking and the rule under it, and a
section name written in capitals in one place and not the other would be two names.

One section is drawn at a time, without its heading — the rail says what the section is,
and a page repeating the word under the item that already says it is a line of nothing.

The rail is drawn only when there is more than one named section. A page with one section,
or with rows and no headings at all, is the page this was before there was a rail: no rail,
the whole list in the page, and the headings drawn as headings.

A narrow window narrows the rail rather than the rows — it is capped at two fifths of the
page — because a truncated section name is legible and a truncated value is not.

Row anatomy: label on the left, control on the right, current value in tabular
figures. The control is a 118x20 rectangle of `ground` behind a `hairline` — recessed
into the page rather than raised off it, because `hairline` is the only depth
mechanism there is. Its whole face is the click target, so the value readout is also
the button.

Four kinds of control, and no more:

- **Choice** — an enum. Either half steps through the list, wrapping. Both halves of a
  two-value choice flip it.
- **Step** — a number. Left goes down, right goes up, and it stops at the end rather
  than wrapping: a font size that jumps from 4 to 72 is worse than one that refuses.
- **Toggle** — a boolean. Both halves flip it; a two-state row has no direction.
- **Chord** — a key binding. Clicking asks for the next key you press, and the row says
  `Press a key` until you press it. `Escape` cancels. If the chord was already spoken
  for, the action that had it goes back to `Unbound` and the row that lost says so.

Hover fills the half a click will take. With no glyphs to read, that fill is the only
thing that says which half is which — and it is why a stepper is two controls wearing
one rectangle rather than one control with two arrows drawn on it.

Rows are scrolled rather than dropped. A page that stops listing settings once the
window is short is a page where a setting cannot be found and nothing says so. A
thickness row appears beside the cursor shape only when the shape has one: a block is
the whole cell and a hollow block is a border on it, so neither has anything for the
number to change.

Every row writes through to the config file as it changes, and the file keeps its
comments — `toml_edit` round-trips, so a hand-written note beside a setting survives
being set from the page.

Keyboard: `Tab` or `Down` enters the page and moves to the next row, `Shift+Tab` or
`Up` to the previous one, `Left` and `Right` (or `Enter`, or `Space`) adjust the row the
keyboard is on, and `Tab` past the last row walks the focus off the end of the page
rather than wrapping. `Escape` does the same, and a second `Escape` closes the tab. The
chord that opened it toggles it, and so does the settings mark.

Unbound keys do not reach the shell while the page is up, and this is the one place the
window departs from what the panel used to do. The panel let a letter through because the
shell was on screen beside it and typing into it was the reason the panel did not cover
it; a page has no prompt behind it, so a letter typed there would reach a shell the user
cannot see and the answer would arrive on a screen that is not being shown. A chord still
goes through, because a chord is the user asking zet itself for something rather than
typing at a prompt. The keyboard reachable from the page is therefore the page's keys and
the app's chords, and nothing else — and `Escape` is the way back.

The sections follow the keyboard rather than being a place the keyboard can be. The rows
are one list, and walking off the end of a section lands on the first row of the next one,
which brings that section to the page with it. That is why the rail needs no key of its
own: a rail that took `Up` and `Down` and asked `Left` and `Right` to pick a section would
need those keys to stop meaning what they mean while it had focus, which is a second mode
inside a page that is deliberately one list. Clicking a section name switches the page and
puts the keyboard back where it was, because there is no row where the click landed.

Focus is drawn as the same hairline in `ink` — the brightest edge the chrome has. `signal`
is the obvious colour for a focus ring and the wrong one: DESIGN.md gives it a 3px by 40px
budget and calls it a lamp, which a border around a 118-pixel control would spend several
times over. Three weights of one hairline, then, and no fourth: the row the keyboard is on
is `ink`, the one under the pointer is `hairline-strong`, and the rest are `hairline`.

## Find

A 32px row pinned above the grid's bottom edge, on `surface-raised`, with a `hairline`
between it and the terminal. It is not part of the grid and does not overlap it: the row's
height comes off the grid's area before the grid is told how big it is, so opening the bar
costs the terminal a row rather than drawing over one.

Inside it: the word `Find` in `ink-dim`, a field of `ground` behind a `hairline-strong`
edge, the query in `ink`, a caret, and a count. The field says what it is for even when it
is empty, which is the whole job of the label — an empty rectangle at the bottom of a
terminal could be anything.

The caret does not blink. Motion is a budget, and this app spends it in one place: the tab
indicator's travel. A second thing moving on screen is a second thing to look at, and the
caret is already the brightest hairline in the row. It is drawn as `ink` at one pixel,
standing a little inside the field.

A query longer than the field is shown from its **end** rather than its beginning. The
caret is where the next character goes, and a caret clipped off the right edge is a field
that looks broken at exactly the moment it is being typed into. The left side is what gets
cut, and it is cut by `char` rather than by byte, because a slice in the middle of a
character is a panic in a paint loop.

The count sits to the right of the field: `3 of 17` in `ink-mid`, or `No results` in
`ink-dim` when there are none. Past a thousand matches it says `1000+` instead of spending
the frame counting to forty thousand — a single letter in a full scrollback is a number
nobody reads and a screen nobody can see through.

### What a match looks like

A match is the theme's `selection` colour laid **over** the cell rather than replacing it,
at half alpha, and the match the arrows are on at all of it. There is one selection colour
in a theme and there is no second highlight colour, which is exactly what two weights of
one colour are for. A cell that a program gave a background keeps it: the mark tints, it
does not paint out.

The grid plane's rule holds throughout — a match is drawn in the theme's colours and never
in the chrome's, even though the bar that found it is chrome. The two planes meet in the
window and nowhere else.

### What a match is

A match is a **logical line**, not a row. A row that wrapped is joined to the one below it
before it is searched, so a word broken across the fold is found, and a match can straddle
the two rows it covers. A row the program ended with a newline is its own line and stops
there — the two look identical on screen and are not the same thing.

Case is insensitive unless the needle has a capital letter in it, anywhere rather than only
at the front: `usb` finds `USB`, and `uSb` finds nothing. A wide character is matched as the
character it is, and its match covers both of the columns it draws in.

### Keys

While the bar is open it owns typing, because it is a text field: every printable character
goes into the query rather than to the shell. Only these keys are its own:

| Key | Does |
|---|---|
| Any text | Appends to the query and searches again |
| `Backspace` | Removes the last character |
| `Enter` | Next match |
| `Shift+Enter` | Previous match |
| `Escape` | Closes the bar and returns the keyboard to the shell |
| `Ctrl+Shift+F` | Toggles it |

The next match scrolls into view if it was not already there, and it is scrolled the
smallest distance that puts it on screen — a match already visible does not move the
viewport under the user. The search wraps at both ends rather than stopping.

## Surfaces the framework gave us

These ship with defaults that belong to no design system, and they get themed from
the palette like everything else:

- Text selection in the grid uses the theme's selection color, and selection in the
  chrome uses `signal` at 20% alpha.
- The scrollbar is a 8px `hairline-strong` thumb on `ground`, growing to 10px on
  hover, with no track and no arrows.
- Focus is a `hairline` in `ink` rather than a `signal` ring. `signal` is the app's one
  lamp and a ring around a control spends its whole budget several times over; the
  brightest hairline says "here" without saying "look at me".
- The find bar is a 32px row pinned above the grid, `surface-raised`, using the same
  hairline language. It is described in [Find](#find).
- The window resize cursor, the IME candidate window anchor, and the drag-and-drop
  overlay all follow the same palette.

## Themes

Eight ship. Three are zet's own and are held to the 4.5:1 ANSI floor. Five are
imported and ship exactly as published.

| Theme | Ground | Note |
|---|---|---|
| zet dark | `#0a0b0d` | Default. Graphite, amber signal, neutral ANSI |
| zet light | `#faf9f7` | Warm paper ground, ink at `#14161a`, signal darkened to `#a35f00` |
| zet contrast | `#000000` | Pure black, pure white ink, every body-text ANSI color at 4.5:1 or better |
| Nord | as published | |
| Gruvbox dark | as published | |
| Tokyo Night | as published | |
| Catppuccin Mocha | as published | |
| Solarized Light | as published | |

The floor is 4.5:1, WCAG AA for normal text, and indices 0 and 8 are exempt from it.
That is not a hole in the rule: a palette's "black" is the colour a program paints a
*background* with, and requiring it to be legible against a near-black ground would
force it to be light, at which point it is no longer black and the programs that use
it as a shadow stop working. Every other index is a colour a program prints text in.

High contrast mode is not a theme. When Windows reports forced colors, zet switches
to `zet contrast` and overrides the chrome palette to pure black, pure white ink,
and the system highlight color, ignoring the user's theme until forced colors turn
off. Themes are the user's choice; forced colors is not — which is why the switch in
the settings page defaults to on and is the only thing that can turn it off, and why
the same is true of reduce motion: a machine that has asked for less movement has
asked for a reason, and the page is where you disagree with it rather than the
absence of a way to.

Text scaling works the same way round. `text_scale` of `0.0` means "follow the
system", which is what makes Windows' own text-size slider work without zet having to
be told about it twice; any other value is the user overruling it, and the chrome's
own type scales with the grid's.
