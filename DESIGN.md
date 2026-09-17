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

**The chrome plane** is zet's: the titlebar, the tab strip, the settings panel, the
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
| `surface-raised` | `#15181c` | Settings panel, popovers, menus |
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
- Text on `surface-raised` is still `ink` or `ink-mid`, never a tinted gray.
- The settings panel shows every value numerically, so the panel is legible as data
  even when the visual preview is off screen.

### Contrast floors

| Pair | Minimum |
|---|---|
| `ink` on `ground` | 12:1 |
| `ink-mid` on `ground` | 6:1 |
| `ink-dim` on `surface` | 4.5:1 |
| `signal` on `surface` | 4.5:1 |
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

**This is not built.** `zet_font::GlyphSpec` has no way to ask for an OpenType
feature, so the numbers are set in Plex Sans's default proportional figures and the
digits in a font size do not line up with the digits in a tab index. Spacing them by
hand would be a lie about the font's own metrics, so it is left and recorded here.

Scale, at 100% DPI:

| Role | Size | Weight | Tracking |
|---|---|---|---|
| Titlebar app name | 12px | 500 | +0.02em |
| Tab index | 13px | 500 active / 400 inactive | 0 |
| Settings section heading | 12px | 500, uppercase | +0.08em |
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
- Numbering is creation order, never renumbered on close. Close #2 of three and
  the remaining tabs are still #1 and #3. A number identifies a session, and
  shuffling numbers under the user is worse than a gap.
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
  tab strip starts looking like everyone else's.
- New tab: a `+` at the end of the run, the footprint of a tab with no name,
  `ink-dim`.
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

- Rail width 48px, cells 36px tall, index centered.
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
state it would appear in is unreachable: `close-tab` quits when the last tab closes,
so a window with no terminals is a window that is on its way out. An empty state for
a state nobody can reach is a line nobody reads.

## Cursor

The cursor is the one element that belongs to both planes, so it follows the theme's
palette and never the chrome's.

- Default: solid block, drawn in the theme foreground with the cell's character
  inverted to the theme background.
- Options: block, bar, underline, hollow block. Thickness is adjustable to 1, 2, or
  3px for bar and underline, which is the accessibility lever for low vision.
- Blink: 530ms on, 530ms off, hard transitions, no fade. Blinking curses if you can
  see the in-between. Off is a first-class setting and is the default when
  reduce-motion is on.
- Unfocused window: hollow block outline, no blink. Never invisible.

## Window chrome

One 40px row holds the app name, the tabs, the drag region, and the caption buttons.
Stacking a titlebar above a tab strip wastes 36px of vertical space to say nothing,
and this is the change that makes zet feel denser than Windows Terminal.

- Caption buttons keep Windows' own metrics: 46px wide, 40px tall, full-height right
  edge, close turning `danger` on hover. Users hit these hundreds of times a day and
  muscle memory is not ours to redesign.
- The drag region is any horizontal gap between the last tab and the caption
  buttons. Double-click maximizes.
- Maximized: the row loses its bottom hairline and the window loses its rounded
  corners, matching how Windows handles a maximized frame.
- The app name is hidden when the tab strip needs the space. Tabs outrank branding.

## Motion

Exactly one authored moment.

**The active indicator travels.** When the active tab changes, the 2px bar moves
from the old tab to the new one over 140ms on an exponential ease-out curve, and the
two indices cross-fade their ink weight over the same duration. The strip itself
does not move, resize, or reflow. One line travels; nothing else reacts.

Everything else is instant:

- Theme changes apply in one frame with no crossfade. A crossfade would show an
  intermediate theme, which is a worse lie than a hard cut.
- The settings panel slides in over 180ms. That is a spatial change, not decoration,
  and it needs the duration to stay legible.
- Tab open and close: the new tab appears already at full size. No scale-in, no
  slide. A tab that animates into existence delays input by exactly as long as the
  animation, which in a terminal is a real cost.
- Reduce motion, read live from the system: the indicator jumps, the panel appears
  instantly, cursor blink stops. Everything still works, nothing moves.

The bar travels because that motion carries the answer to "where am I now." No other
transition in the app is load-bearing, so no other transition exists.

## Settings

A 380px panel anchored to the right edge, on `surface-raised`, separated from the
terminal by a `hairline`. Not a modal. It does not block the terminal, does not dim
it, and does not steal focus from the prompt.

The terminal stays visible behind it and updates live as you change things. Theme,
font, size, opacity, and background all apply to the real terminal the moment they
change, so the preview is not a preview. This is the reason the panel exists at all
rather than a config file alone.

Sections, in order: Appearance, Tabs, Terminal, Keys. Each is a heading at 12px
uppercase with a `hairline` under it, then rows.

Row anatomy: label on the left, control on the right, current value in tabular
figures. The control is a 118x20 rectangle of `ground` behind a `hairline` — recessed
into the panel rather than raised off it, because `hairline` is the only depth
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

Rows are scrolled rather than dropped. A panel that stops listing settings once the
window is short is a panel where a setting cannot be found and nothing says so. A
thickness row appears beside the cursor shape only when the shape has one: a block is
the whole cell and a hollow block is a border on it, so neither has anything for the
number to change.

Every row writes through to the config file as it changes, and the file keeps its
comments — `toml_edit` round-trips, so a hand-written note beside a setting survives
being set from the panel.

Keyboard: `Tab` or `Down` enters the panel and moves to the next row, `Shift+Tab` or
`Up` to the previous one, `Left` and `Right` (or `Enter`, or `Space`) adjust the row the
keyboard is on, and `Tab` past the last row hands the keyboard back to the shell rather
than wrapping. `Escape` does the same, and a second `Escape` closes the panel. The chord
that opened it toggles it.

Focus is drawn as the same hairline in `ink` — the brightest edge the chrome has. `signal`
is the obvious colour for a focus ring and the wrong one: DESIGN.md gives it a 3px by 40px
budget and calls it a lamp, which a border around a 118-pixel control would spend several
times over. Three weights of one hairline, then, and no fourth: the row the keyboard is on
is `ink`, the one under the pointer is `hairline-strong`, and the rest are `hairline`.

Only those keys are the panel's. Every letter and every chord with a modifier still goes
to the shell, because the terminal behind the panel is live and typing into it is the
reason the panel does not cover it. That is what "it does not steal focus from the prompt"
has to mean.

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
the settings panel defaults to on and is the only thing that can turn it off, and why
the same is true of reduce motion: a machine that has asked for less movement has
asked for a reason, and the panel is where you disagree with it rather than the
absence of a way to.

Text scaling works the same way round. `text_scale` of `0.0` means "follow the
system", which is what makes Windows' own text-size slider work without zet having to
be told about it twice; any other value is the user overruling it, and the chrome's
own type scales with the grid's.
