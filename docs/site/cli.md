---
title: Command line
order: 3
---
# Command line

The `furnish` command does what the app does. `furnish --help` and `furnish <command> --help` list all options.

| Command | What it does |
|---|---|
| `furnish list` | The catalog and each app's state: ✓ installed, ↑ update available, · not installed, ★ recommended |
| `furnish install <app>` | Install or update an app or a kit; `--dry-run` shows the steps, `--with`/`--without` choose optional dependencies or kit items, `--previous` installs the app's previous version, `--current` the collection's version of an app held at an earlier one |
| `furnish uninstall <app>` | Uninstall; for a kit, `--only` picks the apps |
| `furnish reinstall <app>` | Apply changed options |
| `furnish upgrade` | Install available updates (`--all` includes apps with automatic updates off) |
| `furnish collection add <url or address>` | Add a collection, or all collections of your organization's furnish address; `--option name=value` answers a question, `--username` (with `--password-stdin` in scripts) logs in to a private HTTPS repository |
| `furnish login <address>`, `logout <address>` | Sign in to your organization's furnish address again, or remove the sign-in |
| `furnish collection list`, `update`, `remove` | Manage collections |
| `furnish option list <collection or app>` | Options, their values and where each value comes from |
| `furnish option set`, `reset` | Change an option, or go back to the collection's value |
| `furnish check [<dir>]` | Check a collection's files before people install from it; shows which resources run on each system |
| `furnish settings` | Show or change automatic updates, and the general download and fallback settings |
| `furnish collection settings <collection>` | Download and fallback settings for one collection: `--fallback ask\|automatic\|off`, `--fallback-max-age <days>`, `--blobs-on-demand <bool>`, `--checkout-depth <commits>`; `--reset` returns to the general settings |

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
   App          Collection  Categories          Description
·  gimp         example     Graphics, Photo     ★ Image editor for photo retouching, image composition and painting.
✓  git          example     Development         Distributed version control system.
✓  htop         example     System, Monitoring  Interactive process viewer for the terminal.
```
