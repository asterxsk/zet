# Packaging

What ships from here:

| File | What it is |
|---|---|
| `zet.iss` | The [Inno Setup](https://jrsoftware.org/isinfo.php) script that builds `zet-<version>-setup.exe` |
| `zet.ico` | The application icon: window, taskbar, shortcut, installer, and Apps & features |
| `zet.svg` | The same mark as vector, for documentation |
| `make-icon.py` | Draws both of the above |

## What the installer does

It installs **for the current user only**, into `%LOCALAPPDATA%\Programs\zet`, and it
never raises a UAC prompt — not for a standard account and not for an administrator.
It writes an uninstaller, `unins000.exe`, into the same directory, and registers itself
under `HKCU` so zet appears in **Settings → Apps → Installed apps** for that user alone.

Three things are offered on the Select Additional Tasks page, and only the first is
ticked by default:

| Task | Default | What it writes |
|---|---|---|
| Start Menu shortcut | on for a fresh install | `[userprograms]\zet` |
| Add zet to my PATH | off | one entry appended to `HKCU\Environment\Path` |
| "Open zet here" in the folder right-click menu | off | `HKCU\Software\Classes\Directory\shell\zet` and `Directory\Background\shell\zet` |

Settings are never touched. `%APPDATA%\zet\config.toml` survives an uninstall, because
reinstalling and losing your configuration is worse than leaving two kilobytes behind.

## Building it

By hand, with Inno Setup 6.3 or newer on `PATH` as `ISCC.exe`:

```sh
cargo build --release -p zet
ISCC.exe /DVersion=0.1.2 /O"dist" /F"zet-0.1.2-setup" packaging\zet.iss
```

`/DVersion` must be the version `Cargo.toml` carries. The release workflow passes the
tag it is releasing, having already checked the two agree.

In CI this runs from `.github/workflows/release.yml`, which compiles the script on every
push to `main` as well — a build that only fails on the day of a release is a build that
has been broken for a month.

## The decisions, and why

**Per-user, no UAC.** `PrivilegesRequired=lowest` makes Setup decline elevation even
when the account could have it, and leaving `PrivilegesRequiredOverridesAllowed` unset
removes the "install for all users" option and the `/ALLUSERS` switch along with it. A
terminal is not a system component; it does not need to write to `Program Files`, and
asking for administrator rights to put a program in your own profile is the kind of
thing users correctly refuse.

One consequence Inno documents and this script cannot undo: if the account *is* an
administrator, Windows marks `unins000.exe` as requiring elevation anyway, because the
installer could have used the privileges it had. A standard account sees no prompt for
either install or uninstall.

**The version is checked, not assumed.** `AppVersion` and `VersionInfoVersion` both come
from `/DVersion`. A binary that reports a different version from the installer that
placed it makes an updater offer the same release forever.

**PATH is written from `[Code]`, not from a `[Registry]` entry.** The obvious
declarative form,

```ini
ValueData: "{olddata};{app}"
```

expands to a leading `;` on a profile that has no user `Path` value yet, and a leading
empty element in `PATH` means *the current directory*. On a terminal, that is a program
being found in whatever folder you happen to be sitting in. `AddToUserPath` builds the
string instead and never emits an empty element. It writes `REG_EXPAND_SZ` through
`RegWriteExpandStringValue`; writing a `REG_EXPAND_SZ` PATH as `REG_SZ` freezes every
`%VARIABLE%` already in it.

Removal is the same argument run backwards, and it cannot be declarative at all:
`uninsdeletevalue` would delete the user's entire `PATH` and `uninsclearvalue` would
blank it. `RemoveFromUserPath` excises the one entry zet added and leaves the rest
alone. It clamps at the first element — published snippets that always delete at
`position - 1` eat the first character of the next entry when zet's directory happens to
be first in `PATH`.

Both directions locate the entry through `FindPathEntry`, which is the one place that
decides what counts as zet's own entry: wrapped in separators so the first and last
elements match like any other, and accepting a trailing backslash. That last part is why
it is one function. The installer tolerates the backslash spelling and declines to add a
second copy of it, so an uninstaller that knew only the spelling the installer writes
would be unable to remove the entry the installer had just left alone, and would report
that there was nothing to remove while the stale entry sat in `PATH`. The function hands
back the length of the entry it matched as well as where it starts, because those two are
not the same number for that spelling.

**The context menu is per-user.** `HKCU\Software\Classes`, not `HKCR`: `HKCR` writes land
in `HKLM` and need the elevation this installer refuses to ask for. Both `Directory` and
`Directory\Background` are registered because right-clicking a folder and right-clicking
the empty space inside one are different gestures with different registry keys.

**No `LicenseFile`.** zet is MIT/Apache-2.0 dual-licensed. Those are permissive licences
that grant rights rather than a EULA that withholds them, so a page demanding the user
click "I accept" before installing would misrepresent what the licence says. The two
licence files and `THIRD-PARTY-NOTICES` are installed next to the binary instead, which
is where they belong: the binary statically links IBM Plex Sans and the IBM Plex Mono
glyphs under the SIL Open Font License, and that licence requires its text to travel
with the font.

**`ChangesEnvironment=yes`.** Without it, the PATH entry is written but
`WM_SETTINGCHANGE` is never broadcast, so nothing already running can see it until the
user signs out. That reads as "the installer did not work", and it is a one-line fix.

**`MinVersion=10.0.17763`.** zet's pty layer runs the shell under ConPTY, which is
Windows 10 version 1809. Installing on anything older would produce a window that opens
and a terminal that never starts.

## The icon

`make-icon.py` holds the geometry and emits both `zet.ico` and `zet.svg`, so the two
cannot drift. It is checked in because the `.ico` is: regenerating is for changing the
mark, not for building.

```sh
python packaging/make-icon.py            # rewrites zet.ico and zet.svg
python packaging/make-icon.py --preview  # and a sheet of every size for judging it
```

The mark is the `#` from the tab strip, with a small square lit in `signal` at its
centre. The preview sheet is written to `D:\Apps\tmp\zet\` and never into the repository,
because the only reason to look at it is to decide whether to change the constants.

The `.ico` reaches the places it is needed from two directions. Inno Setup names it with
`SetupIconFile`, which covers the installer's own window and its entry in Apps & features.
Everything else — the taskbar button, Alt-Tab, Explorer, and the Start Menu shortcut —
reads it out of the **executable**, and that half is `crates/zet/build.rs`: it hands the
file to `winresource`, which compiles a resource section containing the icon and a
version block into `zet.exe` before the linker is finished. A resource cannot be added at
runtime, which is why it takes a build script to do it.

That script is also why `zet.exe` shows a version, a publisher, and a description in
Properties → Details. Those fields are filled from the crate's own `Cargo.toml` rather
than from a number written out twice, so they cannot drift from what `zet --version`
prints.

Note that the strokes thicken below 48px. That is optical sizing, not an inconsistency:
a stroke that is correct at 256px renders at 1.5 pixels at 16px, which is below the
width at which a line holds its colour, and the mark greys out in the size the taskbar
uses most.

## What this does not do

- **The installer is not the update path.** Every release publishes it, and
  `zet --update` does not use it: the updater replaces the running `zet.exe` in place,
  so nothing runs the setup except a person. The installer is for the first install and
  for uninstalling.
- **No code signing.** The installer and the binary are unsigned, so SmartScreen warns
  on first run. `README.md` and `SECURITY.md` say what that costs you.
