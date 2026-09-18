# zet

A terminal multiplexer for Windows, written in Rust, with its own VT engine and a
GPU renderer.

## The problem

Windows developers pick a terminal and then live with its compromises.

Windows Terminal is good and getting better, but it is a large C++/WinRT app whose
rendering and configuration are Microsoft's to decide. Alacritty is fast but has no
tabs and no settings UI. WezTerm has everything and a configuration surface that
needs its own manual. ConEmu is powerful and looks it. None of them let you own the
part that matters.

There is also a correctness problem that only shows up when you use a terminal all
day. Full-screen TUI programs flicker on every repaint unless the terminal honors
synchronized output. Alternate-screen apps leave artifacts on resize. Mouse
reporting, truecolor, and hyperlinks work in some terminals and half-work in others.
You notice this the first time you run an agentic coding tool inside one.

## What zet is

A terminal emulator and multiplexer for Windows 11. Single process, one window,
many terminals.

zet owns its terminal engine end to end: the byte parser, the screen grid, the
scrollback, the reflow, the damage tracking, and the renderer. Nothing about the
character pipeline is borrowed. That is the point of the project and the reason the
rest of it exists.

The renderer is wgpu on DirectX 12, drawing the grid and the window chrome from one
shader pipeline so that a theme change, a background image, or a tab switch costs
one frame and never a repaint storm.

## Who it is for

One person first: a developer on Windows 11 who spends the working day in a
terminal, runs PowerShell 7 and WSL, uses full-screen TUI programs and agentic
coding tools, and has opinions about how the window should look and how fast it
should respond.

The second audience is anyone who has wanted to change something about their
terminal and found they could not.

## What zet promises

**It never flickers.** Not on TUI repaints, not on scroll, not on resize, not on
tab switch, not on theme change. Full-screen programs that use synchronized output
get exactly the behavior the protocol describes: the terminal holds the old frame
until the new one is complete.

**It is fast in the way you feel.** Cold start, keystroke to pixel, scroll latency,
and throughput under heavy output are the numbers that matter, and they get measured
rather than assumed.

**Every TUI rendering mode works.** Alternate screen, synchronized output, mouse
reporting in all its encodings, bracketed paste, focus events, truecolor, OSC 8
hyperlinks, the kitty keyboard protocol. The acceptance test is running Claude Code,
vim, and htop inside zet. If those three are clean, the terminal works.

**It looks deliberate.** One visual world, applied consistently to the tab strip,
the cursor, the settings screen, and every shipped theme.

**You can change what you see.** The theme, the font, the font size, what is painted
behind the grid — flat, a picture, or a gradient — and how opaque the window is, in a
config file with a real settings panel over it that reads and writes the same file.
The grid's own colours are the theme's and only the theme's, which is what lets a
theme be trusted as a whole rather than corrected key by key.

**It respects your machine.** High contrast mode, text scaling, per-monitor DPI,
transparency settings, reduce motion, and GPU-less or remote sessions are all
supported paths, not edge cases that produce a black window.

## What zet is not, in v1

- Not cross-platform. The PTY layer is ConPTY and the shell discovery is the Windows
  registry and the WindowsApps aliases. The renderer is portable but nothing else is.
- Not a screen reader target. zet is fully keyboard-operable and contrast-correct in
  v1. The Windows UI Automation text provider, which is what Narrator and NVDA need
  to read terminal contents aloud, is designed for and deferred.
- Not a shell. zet hosts whatever is installed.
- Not a plugin platform. No Lua, no scripting.

## Success criteria

zet is done for v1 when all of these hold:

1. `cargo build --release` produces a single `zet.exe` that launches in under 300 ms
   to a usable prompt on this machine. `cargo bench -p zet-app --bench latency` prints
   everything of that up to the window — the config read, the shell, and the first
   prompt on the grid — so the figure gets read rather than estimated.
2. PowerShell 7, Windows PowerShell 5.1, cmd, and each installed WSL distribution
   appear as launchable profiles, discovered without being hand-configured.
3. Claude Code, vim, and htop each run full-screen with no visible flicker, correct
   colors, working mouse input, and correct behavior on window resize.
4. Tabs work horizontally and vertically, are numbered from #1 in creation order,
   and the entire tab strip is absent when zero terminals are open.
5. A theme change repaints in a single frame with no intermediate state visible.
6. The config file round-trips: editing it by hand changes the app, and changing the
   app rewrites the file without losing comments.
7. Every theme zet authors itself passes WCAG AA contrast for body text against its
   background. Imported palettes ship unmodified so they look like themselves, and
   the theme picker says so.
8. Running under high contrast mode, at 200% text scaling, and on a machine with no
   usable GPU adapter all produce a readable, working terminal.

## Decisions already made

| Decision | Choice | Why |
|---|---|---|
| Terminal core | Written from scratch in zet | Owning the character pipeline is the reason the project exists |
| Renderer | wgpu on DirectX 12 | Shader freedom for themes, images, gradients, and effects |
| v1 scope | A terminal used daily, not a demo | A half-terminal nobody opens is worth less than a rough one you use |
| TUI rendering | All modes correct and flicker-free | The failure that matters most in daily use |
| Accessibility | Keyboard-complete, contrast-correct, system-mode-correct; UI Automation deferred | Most of the practical value at a fraction of the cost |
| Visual world | Instrument: precision tool | The # numbering reads as designed rather than as a fallback |
| Background | Solid, opacity, images, gradients | Images and gradients are the ceiling people actually want |
| Settings | Config file authoritative, settings panel over it | The only arrangement that ages well |
