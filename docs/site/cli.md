---
title: Command line
order: 7
group: Using Furnish
---
# Command line

The `furnish` command does what the app does. `furnish --help` and `furnish <command> --help` list all options.

On macOS, Homebrew sets up the command when it installs furnish. If you dragged furnish from a disk image, [enable the Terminal command in Settings](install.md#terminal-command-for-the-macos-download).

## Install and update apps

```sh
furnish list
furnish install gimp --dry-run
furnish install gimp
furnish upgrade
```

| Task | Command |
|---|---|
| Select optional dependencies or kit members | `furnish install <app> --with <id>` or `--without <id>` |
| List available revisions | `furnish versions <app>` |
| Install an earlier revision or return to the collection’s version | `furnish install <app> --version <day-or-commit>` or `--current` |
| Remove an app or selected kit members | `furnish uninstall <app>` or `furnish uninstall <kit> --only <id>` |
| Apply changed options | `furnish reinstall <app>` |
| Include apps with automatic updates disabled | `furnish upgrade --all` |

Use `-v` to show output from DSC and the package manager.

## Manage collections

- Add a catalog with `furnish collection add <git-url-or-address>`. Use `--option name=value` to answer a question in a script. For private HTTPS repositories, use `--username` with `--password-stdin`.
- Use `furnish login <address>` to renew an organization sign-in and `furnish logout <address>` to remove it.
- Use `furnish collection list`, `update` and `remove` to manage existing checkouts.
- Preview a revision with `furnish collection update <name> --dry-run`. Apply that comparison with `--reviewed <current> <candidate>`. See [collection tracking rules](collection-updates.md) for branch and tag selectors.
- Run [`furnish check`](validation.md) in a collection repository to validate its files.

## Inspect settings

| Command | What it shows or changes |
|---|---|
| `furnish settings` | Update schedule, fallback settings, language and Furnish’s installed version and channel |
| `furnish collection settings <name>` | Tracking, download and rollback preferences for a collection |
| `furnish option list <collection-or-app>` | Available options, current values and their source. Use `option set` or `option reset` to change them. |
| `furnish self-update --check` | Availability on the selected channel. Use [`furnish self-update`](furnish-updates.md) to install on supported installations. |
