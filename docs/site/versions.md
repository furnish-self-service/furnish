---
title: Package version requirements
order: 24
group: Collection authors
previous: collections
---
# Package version requirements

## Required versions

`Furnish/Package` and `Furnish/Installer` take `versions`, a list of the versions that are acceptable. When a security fix comes out, list the fixed versions, and furnish updates the computers that have an older one:

```yaml
- name: Firefox
  type: Furnish/Package
  properties:
    name: firefox
    versions:
      - ">=128.0.3, <129"
      - "115.15.0esr"
      - regex: '^115\.(1[6-9]|[2-9][0-9])\.'
```

The installed version is acceptable when it matches at least one entry:

| Entry | Matches |
|---|---|
| A fixed version, such as `115.15.0esr` | exactly this version |
| A range, such as `>=128.0.3, <129` | versions for which every comparison holds. A range starts with `>=`, `>`, `<=`, `<`, `=` or `!=`, and commas separate its comparisons |
| `regex: '...'` | versions whose text the regular expression finds a match in. Use `^` and `$` to match the whole text |

Versions compare by the rules of what installs them:

| Installed with | Rules |
|---|---|
| winget | winget's: `1.0` is the same as `1.0.0`, and `1.0-beta` comes before `1.0` |
| Homebrew, `.pkg`, `.dmg`, `.msi`, `.exe` | numbers compare as numbers; `alpha`, `beta`, `pre` and `rc` come before the release, other words such as `esr` after it |
| pacman | `vercmp` |
| apt, `.deb` | `dpkg --compare-versions` |
| dnf, zypper, `.rpm` | rpm's |
| Nix | Nix's `builtins.compareVersions` |

So `1.10` is later than `1.9`, and an epoch such as `1:` makes a version later than every version without one. A release or Debian revision (`-1`, `-0ubuntu1`) only counts when the entry has one too: `>=128.0.3` accepts `128.0.3-1`.

When the installed version isn't acceptable, the app shows "Security update required" and furnish installs the update as soon as it knows, even if the person turned automatic updates off for the app. It doesn't wait for the update interval or for the computer to be idle, only for an install that is already running. furnish installs:

1. the newest version the package manager offers, if it is acceptable;
2. otherwise, with winget, apt, dnf and zypper, which can install a given version, the newest acceptable version they offer;
3. otherwise nothing: the app shows what it needs and what is offered, for example "Needs 128.0.3 or later; winget offers 128.0.1", and [reports](reports.md#reports) record it as a failure.

While an acceptable version is installed, furnish doesn't update it to one that isn't. With `<129`, Firefox 129 waits until the collection accepts it.

For `Furnish/Installer`, the version offered comes from the installer. For files in the collection, `furnish check` reads the version of `.deb` files (with `dpkg-deb`) and of `.rpm` files, reports an error for an unacceptable version, and warns when it cannot read the version. For a URL, it checks the pin and URL offline. During installation furnish evaluates version requirements against the verified download wherever the installer format exposes its version.

furnish never falls back to an earlier version of the app that the current `versions` don't accept (see [When an install fails](app-updates.md#when-an-install-fails)).

Sometimes furnish can't read the installed version: for a package installed outside furnish (see `command`), or one the package manager installed without a version, such as some portable winget packages. furnish then leaves the app as it is and installs nothing over it. The app's page says that furnish can't check its version against `versions`, and where it is installed if it was installed outside furnish. `furnish list` says "version unknown", and [reports](reports.md#reports) record it.

`Furnish/NixOS` takes no `versions`: its packages come from the system's nixpkgs, which decides their version.
