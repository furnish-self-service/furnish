---
title: Command line
order: 3
---
# Command line

The `furnish` command does what the app does. `furnish --help` and `furnish <command> --help` list all options.

On macOS, Homebrew sets up the command when it installs furnish. If you dragged furnish from a disk image, [enable the Terminal command in Settings](index.md#terminal-command-for-the-macos-download).

| Command | What it does |
|---|---|
| `furnish list` | The catalog, with texts in your language (see [Translations](collections.md#translations)), and each app's state: ✓ installed, ↑ update available, · not installed, ★ recommended |
| `furnish install <app>` | Install or update an app or a kit; `--dry-run` shows the steps, `--with`/`--without` choose optional dependencies or kit items, `--previous` installs the app's previous version, `--version <day or commit>` the version of that day or commit and keeps the app there, `--current` the collection's version of an app held at an earlier one |
| `furnish versions <app>` | The versions of an app you can install with `--version`, newest first: day, commit and the versions the collection's file accepted; ✓ marks the installed one |
| `furnish uninstall <app>` | Uninstall; for a kit, `--only` picks the apps |
| `furnish reinstall <app>` | Apply changed options |
| `furnish upgrade` | Install available updates (`--all` includes apps with automatic updates off) |
| `furnish collection add <url or address>` | Add a Git collection, optionally with `--branch NAME` or `--tag-pattern REGEX [--tag-order version\|newest]`, or add the collections from your organization's Furnish address. `--option name=value` answers a question, `--username` (with `--password-stdin` in scripts) logs in to a private HTTPS repository |
| `furnish login <address>`, `logout <address>` | Sign in to your organization's furnish address again, or remove the sign-in |
| `furnish collection list`, `update`, `remove` | Manage collections |
| `furnish option list <collection or app>` | Options, their values and where each value comes from |
| `furnish option set`, `reset` | Change an option, or go back to the collection's value |
| `furnish check [<dir>]` | Check a collection's files before people install from it; shows which resources run on each system |
| `furnish settings` | Show or change automatic updates, the general download and fallback settings, and the fallback language for collection texts (`--fallback-language <tag>`). Also shows furnish's version, how it was installed and whether a new version is out; `--self-update auto\|notify\|off` sets what furnish does with new versions of itself |
| `furnish self-update` | Install the newest version of furnish, if you installed it from the Windows or macOS download. Elsewhere it says how the new version arrives. `--check` only says whether there is one |
| `furnish collection settings <collection>` | Set its update source with `--branch NAME`, `--tag-pattern REGEX [--tag-order version\|newest]`, `--default-branch`, or `--inherit-tracking`. Download and fallback settings include `--fallback ask\|automatic\|off`, `--fallback-max-age <days>`, `--choose-version <bool>`, `--blobs-on-demand <bool>`, `--checkout-depth <commits>`; `--reset` returns to the general settings |

`-v` shows what DSC and the package manager do. In a terminal, furnish asks a collection's unanswered questions; in scripts, it prints the `furnish option set` command that answers them.

```
$ furnish install gimp --dry-run
Installing gimp would:

    skipped        gimp (winget)  (Furnish/Package)
                   Not on this system: [equals(context().os.family, 'Windows')]

  + install        gimp  (Furnish/Package)
                   Install gimp with pacman
                   Run as administrator: pacman -S --noconfirm --needed gimp

$ furnish list
   App          Collection  Categories             Description
✓  firefox      example     Internet               ★ Web browser from Mozilla.
✓  git          example     Development            Distributed version control system.
·  thunderbird  example     Communication, Office  ★ Email, calendar and contacts.
```
