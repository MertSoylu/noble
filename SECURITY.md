# Security Policy

NOBLE reads the login files that AI coding CLIs keep on your machine (for example `~/.claude/.credentials.json`
or `~/.codex/auth.json`) to show your remaining quota, and it runs your shells. Security reports are taken
seriously.

## Supported versions

Only the latest release receives security fixes.

| Version | Supported |
|---|---|
| 1.x (latest) | ✅ |
| older | ❌ |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report it privately through GitHub:
[**Report a vulnerability**](https://github.com/MertSoylu/noble/security/advisories/new)
(Security tab → *Report a vulnerability*).

Please include:

- what the problem is and what an attacker could do with it,
- steps to reproduce, or a proof of concept,
- the NOBLE version (`noble --version`), OS and terminal.

You can expect a first reply within a week. Once the issue is confirmed, a fix is prepared and released, and
you are credited in the release notes unless you prefer otherwise.

## Scope

Examples of what we especially want to hear about:

- a credential or token being sent anywhere other than its own provider, shown on screen or written to a log,
- config, workspace or session files that can make NOBLE run commands you did not ask for,
- escape sequences printed by a program in a pane that can break out of the pane or run commands,
- changes to `~/.claude/settings.json` beyond the hook entries NOBLE adds and removes.
