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
| `ink-dim` | `#5a626b` | Inactive tab index, disabled controls |
| `signal` | `#ffa62b` | The active marker. Indicators and focus rings only |
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

A tab is a number, not a label.

```
 horizontal, one row shared with the titlebar
+---------------------------------------------------------------+
| zet   #1   #2   #3        (drag region)        _   []   x     |
|       ~~                                                      |
+---------------------------------------------------------------+
```

- The index renders as `#` at 70% of the number's size, then the number. The
  `#` is part of the mark, not a prefix that disappears. It is what makes the
  strip read as a set of numbered channels.
- Numbering is creation order, never renumbered on close. Close #2 of three and
  the remaining tabs are still #1 and #3. A number identifies a session, and
  shuffling numbers under the user is worse than a gap.
- Single digit up to #9, then `#10` and beyond. Tab width grows to fit.
- Active tab: a 2px `signal` bar on the bottom edge plus `ink` at weight 500.
- Inactive tab: `ink-dim`, weight 400, no bar, no background.
- Hover: index moves to `ink-mid`. The indicator bar previews at `signal-dim`. No
  background fill, ever. Filling a hovered tab is the single most common way a tab
  strip starts looking like everyone else's.
- New tab: a `+` at the end of the run, same footprint as a tab, `ink-dim`.

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
- The divider between the rail and the grid is a `hairline`.
- Active tab carries the same 2px `signal` bar, moved to the left edge. Same
  language, rotated.
- The rail has no scrollbar. Overflowing tabs compress the cell height to a 24px
  floor, then the rail scrolls under the wheel.

### Zero tabs

When no terminal is open the strip does not collapse to a thin line and does not
show an empty state message. It is simply absent. The window is a titlebar, a
hairline, and an empty grid holding one centered line:

```
  No terminals.  Ctrl+T for PowerShell 7,  Ctrl+Shift+T to choose a profile.
```

The whole row that held tabs is gone. The window is 40px shorter. This is the
brief's requirement and it should feel like the app got lighter, not like something
is missing.

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
figures. Controls are segmented buttons for enums, a hairline-bordered stepper plus
a numeric readout for sizes, a slider with a numeric readout for opacity, and a
swatch grid for colors. Every control reports its value as a number, so the panel
can be read as a spec sheet.

Keyboard: the panel is fully reachable with Tab, arrow keys adjust values, Escape
closes. It never traps focus.

## Surfaces the framework gave us

These ship with defaults that belong to no design system, and they get themed from
the palette like everything else:

- Text selection in the grid uses the theme's selection color, and selection in the
  chrome uses `signal` at 20% alpha.
- The scrollbar is a 8px `hairline-strong` thumb on `ground`, growing to 10px on
  hover, with no track and no arrows.
- Focus rings are a 2px `signal` outline offset by 1px, on every focusable control
  including the tab strip and the grid itself.
- The find bar is a 32px row pinned above the grid, `surface-raised`, using the same
  hairline language.
- The window resize cursor, the IME candidate window anchor, and the drag-and-drop
  overlay all follow the same palette.

## Themes

Eight ship. Three are zet's own and are held to the 4.5:1 ANSI floor. Five are
imported and ship exactly as published.

| Theme | Ground | Note |
|---|---|---|
| zet dark | `#0a0b0d` | Default. Graphite, amber signal, neutral ANSI |
| zet light | `#faf9f7` | Warm paper ground, ink at `#14161a`, signal darkened to `#a35f00` |
| zet contrast | `#000000` | Pure black, pure white ink, every ANSI color at 7:1 or better |
| Nord | as published | |
| Gruvbox dark | as published | |
| Tokyo Night | as published | |
| Catppuccin Mocha | as published | |
| Solarized Light | as published | |

High contrast mode is not a theme. When Windows reports forced colors, zet switches
to `zet contrast` and overrides the chrome palette to pure black, pure white ink,
and the system highlight color, ignoring the user's theme until forced colors turn
off. Themes are the user's choice; forced colors is not.
