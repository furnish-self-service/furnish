---
title: Packages and installers
order: 22
group: Collection authors
previous: collections
---
# Packages and installers

## Furnish/Package

Installs a package with a system, per-user or collection package manager. The default on Windows is winget. On macOS and Linux, Furnish checks pacman, apt, dnf, zypper, brew and nix in that order.

| Property | Meaning |
|---|---|
| `name` | The default package name (required) |
| `packageManager` | Select a specific package manager |
| `feed` | HTTPS interface URL for the `0install` manager |
| `namesByDistribution` | Package names keyed by Linux distribution ID, including IDs used by your organization |
| `namesByManager` | Package names keyed by package manager, such as `brew`, `pacman` or `winget` |
| `flake` | Collection-relative folder containing `flake.nix` for a Nix profile install |
| `nixpkgs` | An explicit flake for a Nix profile install. Profile installs default to the registry’s `nixpkgs`. Declarative installs use the configuration’s locked nixpkgs. |
| `command` | A program the package provides, for example `claude`. If the package manager doesn't have the package but the program is there, the app shows as installed outside furnish |
| `versions` | The versions that are acceptable (see [Required versions](versions.md#required-versions)) |

Furnish first looks up the Linux distribution's `ID` from `os-release`, then each entry in `ID_LIKE` in the order the distribution lists them. If none match, it looks up the selected package manager in `namesByManager`, then falls back to `name`. macOS and Windows start with the package manager lookup.

```yaml
- name: Company tools
  type: Furnish/Package
  properties:
    name: company-tools
    namesByDistribution:
      acme-linux: acme-workstation-tools
      debian: company-tools-deb
    namesByManager:
      brew: company-cli
      winget: Acme.CompanyTools
```

To ship a Nix package from the collection, use `flake` with the folder containing its `flake.nix`. Commit its `flake.lock` to pin its inputs. Furnish reads the folder from its checkout, including files deferred by sparse checkout. Nix needs access to any external flake inputs.

```yaml
- name: tray icon
  type: Furnish/Package
  properties:
    name: openvpn3-indicator
    packageManager: nix
    flake: files/nix/openvpn3-indicator
```

Furnish installs the named package into the Nix profile and compares the installed flake’s content hash on subsequent checks. Editing a file in the folder, including its lock file, makes the app eligible for update even when its version string stays the same.

When this step selects Nix on a machine with a configured Furnish apps input, the resolved package name is an attribute of the configuration’s nixpkgs, such as `jq` or `kdePackages.kdenlive`. Furnish can use the same step for NixOS or Home Manager. Supply `flake`, `nixpkgs` or `versions` when the app must use a particular source or release. Those steps use a Nix profile. If an update adds a source or version constraint to a package that Furnish already installed in the configuration, uninstall the app and install it again to move it to the profile. [NixOS configuration](nixos.md#installing-apps-into-the-configuration)

When migrating an existing collection, move each per-manager property into `namesByManager`. For example, replace `pacmanName: libreoffice-fresh` with `namesByManager: { pacman: libreoffice-fresh }`.

Many tools are also installed some other way: with npm or pip, with the vendor's own installer, or with another package manager. With `command`, such an app shows as installed outside furnish, and furnish neither installs it a second time nor updates or removes it. furnish looks for the program on `PATH`, then in `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin` (on Windows, in `~\.local\bin` and npm's folder in `%APPDATA%`), and never runs it. If the package manager is missing, the app shows as installed when the program is there, and as needing the package manager when it isn't.

On macOS, Homebrew packages also recognize vendor-installed apps without a `command`. For example, a package named `viscosity` recognizes `/Applications/Viscosity.app` as installed outside furnish. Furnish checks the app bundles named by the cask in `/Applications` and `~/Applications`. These apps satisfy dependencies and keep their original update and uninstall method.

After a package step succeeds, Furnish remembers its manager and resolved package name for this app and step. For Zero Install it also remembers the feed. Installing a higher-priority manager later keeps status checks, updates, and removal with the original manager. If that manager is missing, restore it to manage the installed package. A successful uninstall clears the record so a fresh install can select a manager again. Existing installations without a record use normal manager selection.

### Per-user tools with mise

Use mise for tools distributed through its registry or a backend such as GitHub, Aqua or pipx. To require mise on Linux, macOS and Windows, set `packageManager: mise` and use the tool ID as `name`. Supply version requirements in `versions`. For example, this step installs the GitHub release of Kargo in the signed-in user's account.

```yaml
- name: Install Kargo
  type: Furnish/Package
  properties:
    packageManager: mise
    name: github:akuity/kargo
    command: kargo
```

To offer mise as a fallback, use a portable `name` and put its tool ID in `namesByManager.mise`. For example, `name: jq` with `namesByManager: { brew: jq, mise: 'aqua:jqlang/jq' }` uses Homebrew on a Mac with both managers and mise on a Windows machine with winget and mise. A matching Linux distribution name selects the usual system manager first. Otherwise, when mise is available, Furnish picks the first installed manager with an explicit name, in the priority order shown under [target profiles](validation.md#check-target-profiles-in-ci). Without mise the usual system selection applies. Use `packageManager: mise` when the tool is available only through mise.

The built-in manager reserves the ID `mise`. If your collection declares its own script manager under that name, remove the declaration to use the built-in manager, or rename the script manager and the steps that reference it.

An app with a step that selects mise runs as the signed-in user, including apps in admin-preset collections. Put machine-wide prerequisites in a separate dependency if they require the Furnish service. Collection authors can check this selection across platforms with [target profiles](validation.md#check-target-profiles-in-ci).

### Per-user apps with Zero Install

Install a [Zero Install](https://docs.0install.net/) application for the signed-in user.
Install Zero Install first, or add an app providing the `0install` command as a dependency.

```yaml
- name: Install VLC with Zero Install
  type: Furnish/Package
  properties:
    packageManager: 0install
    feed: https://apps.0install.net/gui/vlc.xml
    name: vlc
```

`feed` is the HTTPS interface URL. Zero Install verifies the feed and its implementations
using its configured trust settings. On Linux and macOS, `name` is the command-line launcher
created by `0install add`. A `namesByManager.0install` entry overrides that launcher name, including on Linux. Furnish checks the registered feed before accepting or removing
a launcher with that name. On Windows, Zero Install uses the feed as the identity in **My Apps**
and applies the feed’s standard desktop integrations. Choose a feed that provides an
implementation for the target system.

The preview shows the commands without downloading the app. Furnish manages whether the
registration exists. Zero Install handles newer feed versions and its shared implementation
cache. Uninstalling removes the registration and its launcher or desktop integration.

To offer Zero Install as a fallback, omit `packageManager`, keep `feed`, and give the launcher name in `namesByManager.0install`. For example, `namesByManager: { brew: vlc, 0install: vlc }` uses Homebrew on a Mac with both managers and Zero Install on a Windows machine with winget and Zero Install. A matching Linux distribution name takes precedence when its system manager is available.

### Package managers supplied by a collection

An admin-preset collection can ship a package manager for a system Furnish does not handle itself. Commit the script to the collection and give it a name in `collection.yaml`. The name is local to that collection. Furnish runs `.sh` with `/bin/sh` on Unix and `.ps1` with Windows PowerShell on Windows. For another language, set `interpreter` to an executable name, such as `python3` for a script named `managers/company.py`. That manager is available wherever Furnish finds the interpreter.

```yaml
packageManagers:
  apk:
    script: managers/apk.sh
    requiresCommand: apk
```

`requiresCommand` is optional. When specified, Furnish checks for the executable on `PATH` before advertising the manager in `variables('furnish.host').packageManagers`. A step can then select it explicitly:

```yaml
- name: Install ripgrep on Alpine
  type: Furnish/Package
  condition: "[contains(variables('furnish.host').packageManagers, 'apk')]"
  properties:
    name: ripgrep
    packageManager: apk
```

With no explicit selection, Furnish tries its built-in managers first. If none are present and exactly one collection manager is available, it uses that manager. If several are available, set `packageManager` on the step. Collection managers use the same distribution and manager name lookups as system managers.

The script receives one operation as its first argument and one UTF-8 JSON object on stdin. For example, an install request might contain `{"name":"ripgrep","packageManager":"apk","_exist":true}`. Return one JSON object on stdout containing the resolved `name` and boolean `_exist`. Send progress and diagnostic messages to stderr. A nonzero exit fails the operation.

| Operation | What the script does |
|---|---|
| `get` | Read the current state. Include `version` when known. |
| `test` | Read the current state and include `_inDesiredState`, a boolean indicating whether it satisfies the request. |
| `what-if` | Return the expected state without making changes. Put human-readable steps in `_metadata.whatIf` to show them in the preview. |
| `set` | Apply the request idempotently and report the resulting state. Install or update for `_exist: true`, remove for `false`. |

For apps with [required versions](versions.md#required-versions), Furnish sends `reportVersions` (`latest` or `all`) when examining available versions. Return `latestVersion`, `versionScheme`, and `via`, and set `supportsVersionPin: true` if the tool can install an exact version. For such a manager, return `availableVersions` when asked for all. If the request includes `targetVersion`, install that exact version or return an error.

`furnish check` validates manager declarations and their script files without running them. Only the owning preset collection can use its scripts. A system install runs from the service's copy of that collection. A per-user install runs under that user's account. The [Alpine example](https://github.com/furnish-self-service/furnish/blob/main/docs/examples/apk.sh) shows the complete script.

## Furnish/Installer

Installs a `.deb`, `.rpm`, `.pkg`, `.dmg`, `.msi` or `.exe` from the collection or a vendor download. To ship the installer in the collection, set `file` to its path. Furnish supports large files tracked with Git LFS. The desktop installers and Nix packages provide Git LFS. Install it separately when using the standalone Linux CLI. Collections served through a Furnish address also need the [endpoint’s LFS configuration](endpoint-access.md#git-lfs-downloads).

```yaml
- name: Company VPN
  type: Furnish/Installer
  properties:
    file: installers/vpn.msi
```

`file` is a path inside the collection. furnish refuses a path that leads outside it, also through a symbolic link, and the resource refuses to install a symbolic link. `versions` lists the versions that are acceptable (see [Required versions](versions.md#required-versions)).

To use a vendor download, set `url` and `sha256`. For example, an app that depends on Homebrew can name this installer as a dependency:

```yaml
- name: Homebrew
  type: Furnish/Installer
  condition: "[equals(context().os.family, 'macOS')]"
  properties:
    url: https://github.com/Homebrew/brew/releases/download/7.0.14/Homebrew.pkg
    sha256: 8d7adb3cf2cd26a6fcdc3b93570e7e1ce79e832d780a52e5ba672a303dbf3789
    packageId: sh.brew.homebrew
    command: brew
```

Use an HTTPS URL whose path ends in the installer extension, and pin the 64 hexadecimal digits of the file’s SHA-256. Update the URL and checksum together when selecting another release. Listing apps, checking a collection, and previewing an install leave the download until installation. Furnish verifies the bytes before handing them to the usual platform installer. A checksum mismatch stops the step before elevation.

Set `command` to a program the installer provides when users may already have installed it another way. Furnish checks the installer's receipt, bundle or registered product first. If that identity is absent and the executable exists, Furnish shows its path as installed outside Furnish and leaves updates and removal to the original installation method. For Homebrew installed with `install.sh`, `command: brew` also finds the executable in `/opt/homebrew/bin` or `/usr/local/bin` when those directories are missing from the desktop app's PATH.

For macOS packages, furnish can read a single receipt identifier from the verified download. Set `packageId` to choose a receipt explicitly or detect an existing installation before downloading the package. For disk images, set `app` to the bundle name. On Windows, `productCode` identifies an MSI before download, and an EXE uses `displayName` and `installArgs` as it does for a collection file. When the package identity comes from the installer, furnish uses the cached file for removal and retrieves it again if necessary.
