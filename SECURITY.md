# Security Policy

## Reporting a vulnerability

Report privately through GitHub's
[private vulnerability reporting](https://github.com/asterxsk/zet/security/advisories/new)
on this repository. Do not open a public issue for a security problem.

If you cannot use that form, email **112053934+asterxsk@users.noreply.github.com**.

Please include: the zet version (`zet --version`), your Windows build, what you did, and what
happened. A proof of concept is welcome; a working exploit is not required.

**What to expect.** An acknowledgement within 7 days. If the report is valid, a fix and a
GitHub Security Advisory, with credit unless you ask otherwise. If it is not, an explanation of
why. This is a spare-time project, so those are targets rather than guarantees.

## Supported versions

Only the latest release receives fixes. zet is pre-1.0 and the terminal is not yet usable as a
daily driver, so there are no maintenance branches.

## What counts as a security issue here

zet is a terminal emulator. It parses untrusted bytes, runs processes, and replaces its own
executable. In scope:

- **Escape-sequence parsing.** Memory unsafety, panics reachable from terminal output, or
  arbitrary code execution via a crafted escape sequence. This is the highest-value target:
  a terminal parses bytes from every program you run.
- **The update path.** Anything that lets a party other than the release process cause zet to
  download or run a binary: a checksum bypass, a redirect to an attacker-controlled host, a
  path-traversal in the extraction step, or a way to make a forged release look signed.
- **Command execution.** Anything that turns terminal output into executed code without the
  user asking for it — a hyperlink or OSC handler that runs a command, a paste that escapes a
  bracketed-paste guard, a title string that reaches a shell.
- **Sandbox and privilege boundaries.** A child process escaping the job object that is supposed
  to die with the session, or zet itself doing something it should not have the rights for.
- **Denial of service that is not just slowness.** A remote party making zet hang or crash by
  printing bytes.

Out of scope:

- Anything requiring an attacker who already runs code as you. A terminal deliberately runs
  programs you tell it to run; that is the feature.
- The behaviour of programs you run inside zet. Report those to their authors.
- Missing hardening with no demonstrated impact.
- Anything in a dependency that upstream has not yet fixed — report it upstream, and tell us so
  we can pin or patch.

## Release integrity

Releases are built by GitHub Actions from a signed tag in this repository and published with a
`SHA256SUMS` file alongside the archive. The updater refuses an archive whose checksum does not
match. The build pipeline is in [`.github/workflows/release.yml`](.github/workflows/release.yml)
and is readable in full.

**Archives are not code-signed.** Windows SmartScreen and some corporate endpoint tools will
warn on first run, because zet ships no Authenticode signature. That is a known limitation, not
a compromise. A checksum proves the download matches what the release published; it does not
prove who published it. Tag protection and a GitHub account with two-factor authentication are
what stand in for signature verification today.
