# Privacy Policy

**Effective date:** 17 September 2026
**Applies to:** zet ("the application"), a terminal emulator for Windows distributed from
<https://github.com/asterxsk/zet>.

zet is a local desktop application. It has no user accounts, no server of its own, and no
telemetry. This policy describes the one situation in which the application contacts the
network on its own, and what it writes to your disk.

## Summary

- zet does **not** collect, transmit, or store telemetry, analytics, crash reports, or usage data.
- zet does **not** have accounts, logins, or any server it operates.
- zet **does** make one kind of outbound request: an update check against the public GitHub
  Releases API. It can be turned off.
- Everything your shell prints stays on your machine.

## What stays on your machine

zet is a terminal. It reads the output of the programs you run inside it, parses that output,
and draws it in a window. All of that happens locally, in memory, on your computer. Terminal
contents, command history, scrollback, and anything printed by the programs you run are never
sent anywhere by zet.

Some of the programs you run inside zet will themselves use the network — a package manager, a
git remote, an agentic coding tool. Those programs have their own privacy practices and their
connections are theirs, not zet's. zet does not inspect, log, or forward them.

## The update check

**What it is.** `zet --check-update` asks GitHub whether a newer version has been published and
prints the answer. It makes no other request, downloads nothing, and changes nothing on disk.

`zet --update` is the same check and then the download: if there is a newer version for this
machine it fetches the archive, checks it against the digest the release published, and puts it
where the running binary is. The new version takes effect the next time zet starts. Nothing is
written until the download has matched its published digest.

**What zet does not do.** It does not make this request on its own. There is no check on launch,
no timer, and no background thread that talks to GitHub: the request happens when you type the
command, and at no other time. A terminal that opens a socket because it was launched is a
terminal that has to be trusted further than this one asks to be.

**Where it goes.** `api.github.com`, operated by GitHub, Inc. (a subsidiary of Microsoft
Corporation).

**What is sent.** The HTTP request itself necessarily discloses:

| Data | Why |
|---|---|
| Your IP address | Unavoidable in any TCP connection; GitHub sees it |
| A `User-Agent` string identifying zet and its version | GitHub's API requires one, and it lets us tell releases apart in aggregate |
| The `Accept` header GitHub's API requires | Protocol requirement |

**What is not sent.** No identifier for you or your machine is generated or included. zet does
not send a device ID, an installation ID, a username, a hostname, a machine GUID, terminal
contents, command history, or a list of installed shells. There is no analytics payload.

**What GitHub does with it.** GitHub's handling of that request is governed by the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-privacy-statement).
GitHub is an independent controller for this data; we do not receive it and cannot retrieve it.

**Downloads.** When you run `zet --update` and there is something to install, the release archive
is downloaded from `github.com` / `objects.githubusercontent.com`. The same disclosure applies.
Nothing else in zet downloads anything, and nothing is ever downloaded without you typing that
command.

**Turning it off.** There is nothing to turn off, and that is the design rather than an omission.
An earlier draft reserved a `[update] check-on-launch` key in `config.toml` for an automatic
check; the key was removed before the first release, because the automatic check is not something
this program should do. zet has no `[update]` section, the settings panel has no update row, and
the binary accepts `-d`/`--directory`, `-h`/`--help`, `-V`/`--version`, `--check-update`, and
`--update`, refusing any other argument by name. So the only request zet can make is one you
typed, and the only file it will rewrite is its own binary.

## What zet writes to your disk

| Location | Contents | Why |
|---|---|---|
| `%APPDATA%\zet\config.toml` | Your settings: theme, font, shell profiles, keybindings | To remember your configuration |
| `%LOCALAPPDATA%\zet\logs\` | Diagnostic logs, if you enable them. Off by default | To help you diagnose a problem you report |
| Beside `zet.exe` | An update, while it is being put in place: the verified download as `zet.exe.new`, then the binary it replaced as `zet.exe.old` | To replace a running binary, which Windows allows only by renaming it |

The logs row is not in use: nothing writes a log. The update row is, when you run `zet --update`.
The new binary has to be staged beside the running one rather than in a cache directory, because
a rename across volumes is a copy and a delete — neither atomic nor able to replace an open file
— and the displaced `zet.exe.old` is deleted the next time zet starts, by which point nothing is
holding it.

zet does not read files outside those directories and the ones you point it at — for example a
theme file you choose, or a working directory you open.

## Children

zet is a developer tool. It is not directed at children and we do not knowingly collect
information from anyone, including children.

## Your rights

Because zet collects no personal data and operates no server, there is no personal data for us
to export, correct, or erase. The only personal data involved anywhere in the update path is
held by GitHub, and requests about it go to GitHub under the statement linked above.

If you believe zet is handling data in a way this policy does not describe, please open an issue
at <https://github.com/asterxsk/zet/issues> or email the address below.

## Changes to this policy

If zet ever adds anything that transmits data — crash reporting, a package index, an account —
this policy will be updated before that version ships, and the change will be called out in
[CHANGELOG.md](CHANGELOG.md). The effective date above is the date of the last change.

## Contact

**asterxsk** — <112053934+asterxsk@users.noreply.github.com>

For security vulnerabilities, please use the process in [SECURITY.md](SECURITY.md) instead of a
public issue.
