---
title: Writing a collection
order: 4
---
# Writing a collection

A collection is a git repository with this layout:

```
collection.yaml          # the collection's icon and the questions it asks
apps/<app>.dsc.yaml      # one file per app
kits/<kit>.yaml          # apps that are installed together
locales/<lang>.yaml      # translations, one file per language
icons/                   # images the files above refer to
```

furnish clones the repository for each user and pulls changes at start and every 30 minutes. It downloads the history's commits, but file contents only for the current version; earlier versions and installer files come when they are needed. The [example collection](https://github.com/furnish-self-service/self-service-example) has real apps with kits, translations, icons, dependencies and a question that recommends apps, and its Acme apps set up configuration with `Furnish/Directory`, `File`, `Line`, `Value` and `Command`.

## Apps

An app is a [DSC v3](https://learn.microsoft.com/powershell/dsc/overview) configuration document. Its file name without `.dsc.yaml` is the app's ID, which commands, the policy and dependencies use. furnish shows what `metadata.furnish` says in the catalog and runs `resources` to install the app:

```yaml
metadata:
  furnish:
    name: GIMP                # shown instead of the ID; optional
    description: Image editor for photo retouching, image composition and painting.
    categories: [Graphics, Photo]
    icon: icons/gimp.svg
resources:
  - name: GIMP
    type: Furnish/Package
    properties:
      name: gimp
      namesByManager:
        winget: GIMP.GIMP.3
```

furnish ships resources for installing software (`Furnish/Package`, `Furnish/Installer`, and on NixOS `Furnish/NixOS`) and for configuring it (see [Configuration](#configuration)), and apps can use any other DSC resource too (see [Other DSC resources](#other-dsc-resources)). furnish skips a resource whose `condition` is false, and hides an app that has no resource for the system. To uninstall, it runs the same resources in reverse order with `_exist: false`, unless the app has [removal steps](#removal-steps).

A step fails when its resource fails. This includes a command that exits with a code other than the expected one, also where the resource itself reports success, like `RunCommandOnSet`. The install stops at that step. After the last step, furnish tests the app. If a resource is still not in its desired state, the install fails and names that resource.

### Changes that need a new login session

When installation changes the environment that a person receives at login, Steps shows a notice before installation. After a successful change, the desktop and CLI ask the person to sign out of the computer and sign back in.

Furnish recognizes changes to Home Manager’s `home.sessionPath` and `home.sessionVariables` and content changes made by `Furnish/File`, `Furnish/Line`, and `Furnish/Block` to standard Unix login profiles such as `~/.profile` and `~/.zprofile`. For a package or custom installer whose changes require a new session, declare this at app level:

```yaml
metadata:
  furnish:
    requiresNewSession: true
```

With this declaration a successful install or update that changes any step produces the notice. An already up-to-date install produces no new notice.

### Host information

Resource steps and dependencies can use the DSC variable `variables('furnish.host')` to choose an install method for the current computer.

| Field | Value |
|---|---|
| `distribution` | Linux distribution ID from `os-release`, such as `fedora`, or an empty string when unavailable |
| `distributionLike` | Ordered list of Linux distribution IDs from `ID_LIKE`, or an empty list when unavailable |
| `packageManagers` | List of supported package managers available to the account running Furnish |
| `nixSetup.system` | Boolean. The NixOS configuration is ready to accept apps through Furnish's module |
| `nixSetup.home` | Boolean. The requesting person's Home Manager configuration is ready to accept apps through Furnish's standalone or NixOS-integrated module |

Both setup fields are `false` on Windows. On macOS only the home destination can be configured. Installing Nix or Home Manager alone leaves the corresponding field `false`. Each field describes readiness independently of the preferred install destination.

For a step specific to Fedora and its derivatives, check both the exact ID and its parents:

```yaml
condition: "[or(equals(variables('furnish.host').distribution, 'fedora'), contains(variables('furnish.host').distributionLike, 'fedora'))]"
```

To prefer Nix wherever it is available, give the Nix step this condition and set `packageManager: nix`. Use the negated condition for the alternative step:

```yaml
condition: "[contains(variables('furnish.host').packageManagers, 'nix')]"
```

### Installing for one person

Set `metadata.furnish.perUser: true` when an app writes files in a person's home folder or calls a tool that stores settings in their account. This includes importing a VPN profile for that person. By default, the furnish service installs apps from an administrator's preset collection as root on Unix or SYSTEM on Windows. In that case, a command can succeed while configuring the service's account.

```yaml
metadata:
  furnish:
    description: Create a private folder for your SSH configuration.
    categories: [Tools]
    perUser: true
resources:
  - name: Personal SSH configuration folder
    type: Furnish/Directory
    properties:
      path: ~/.ssh
      private: true
```

For an administrator's preset collection, a personal app can also include a [`Furnish/NixOS` step](#furnish-nixos). When the step requires a system rebuild, furnish asks the service to apply that step. With standalone Home Manager, the activation runs as the person. If someone chooses an older app version, its privileged step must still match the declaration in the current preset collection.

If an install option selects between a personal and a system-wide install, declare it as a boolean app parameter and set `perUser: "[parameters('userInstall')]"`. Use the same option in the steps that select the install location.

### Furnish/Package

Installs a package with a system, per-user or collection package manager. The default on Windows is winget. On macOS and Linux, Furnish checks pacman, apt, dnf, zypper, brew and nix in that order.

| Property | Meaning |
|---|---|
| `name` | The default package name (required) |
| `packageManager` | Use this package manager, not the first one found |
| `namesByDistribution` | Package names keyed by Linux distribution ID, including IDs used by your organization |
| `namesByManager` | Package names keyed by package manager, such as `brew`, `pacman` or `winget` |
| `nixpkgs` | An explicit flake for a Nix profile install. Profile installs default to the registry’s `nixpkgs`. Declarative installs use the configuration’s locked nixpkgs. |
| `command` | A program the package provides, for example `claude`. If the package manager doesn't have the package but the program is there, the app shows as installed outside furnish |
| `versions` | The versions that are acceptable (see [Required versions](#required-versions)) |

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

When this step selects Nix on a machine with a configured Furnish apps input, the resolved package name is an attribute of the configuration’s nixpkgs, such as `jq` or `kdePackages.kdenlive`. Furnish can use the same step for NixOS or Home Manager. Supply `nixpkgs` or `versions` when the app must use a particular source or release. Those steps use a Nix profile. If an update adds a source or version constraint to a package that Furnish already installed in the configuration, uninstall the app and install it again to move it to the profile. [NixOS configuration](admins.md#installing-apps-into-the-configuration)

When migrating an existing collection, move each per-manager property into `namesByManager`. For example, replace `pacmanName: libreoffice-fresh` with `namesByManager: { pacman: libreoffice-fresh }`.

Many tools are also installed some other way: with npm or pip, with the vendor's own installer, or with another package manager. With `command`, such an app shows as installed outside furnish, and furnish neither installs it a second time nor updates or removes it. furnish looks for the program on `PATH`, then in `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin` (on Windows, in `~\.local\bin` and npm's folder in `%APPDATA%`), and never runs it. If the package manager is missing, the app shows as installed when the program is there, and as needing the package manager when it isn't.

On macOS, Homebrew packages also recognize vendor-installed apps without a `command`. For example, a package named `viscosity` recognizes `/Applications/Viscosity.app` as installed outside furnish. Furnish checks the app bundles named by the cask in `/Applications` and `~/Applications`. These apps satisfy dependencies and keep their original update and uninstall method.

#### Per-user tools with mise

Use mise for tools distributed through its registry or a backend such as GitHub, Aqua or pipx. To require mise on Linux, macOS and Windows, set `packageManager: mise` and use the tool ID as `name`. Supply version requirements in `versions`. For example, this step installs the GitHub release of Kargo in the signed-in user's account.

```yaml
- name: Install Kargo
  type: Furnish/Package
  properties:
    packageManager: mise
    name: github:akuity/kargo
    command: kargo
```

To offer mise as a fallback, use a portable `name` and put its tool ID in `namesByManager.mise`. For example, `name: jq` with `namesByManager: { brew: jq, mise: 'aqua:jqlang/jq' }` uses Homebrew on a Mac with both managers and mise on a Windows machine with winget and mise. A matching Linux distribution name selects the usual system manager first. Otherwise, when mise is available, Furnish picks the first installed manager with an explicit name, with mise last. Without mise the usual system selection applies. Use `packageManager: mise` when the tool is available only through mise.

The built-in manager reserves the ID `mise`. If your collection declares its own script manager under that name, remove the declaration to use the built-in manager, or rename the script manager and the steps that reference it.

An app with a step that selects mise runs as the signed-in user, including apps in admin-preset collections. Put machine-wide prerequisites in a separate dependency if they require the Furnish service. Collection authors can check this selection across platforms with [target profiles](#check-target-profiles-in-ci).

#### Package managers supplied by a collection

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

With no explicit selection, Furnish tries its built-in managers first. If none are present and exactly one collection manager is available, it uses that manager. If several are available, set `packageManager` on the step. The same `namesByDistribution` and `namesByManager` rules apply whichever manager handles it.

The script receives one operation as its first argument and one UTF-8 JSON object on stdin. For example, an install request might contain `{"name":"ripgrep","packageManager":"apk","_exist":true}`. Return one JSON object on stdout containing the resolved `name` and boolean `_exist`. Send progress and diagnostic messages to stderr. A nonzero exit fails the operation.

| Operation | What the script does |
|---|---|
| `get` | Read the current state. Include `version` when known. |
| `test` | Read the current state and include `_inDesiredState`, a boolean indicating whether it satisfies the request. |
| `what-if` | Return the expected state without making changes. Put human-readable steps in `_metadata.whatIf` to show them in the preview. |
| `set` | Apply the request idempotently and report the resulting state. Install or update for `_exist: true`, remove for `false`. |

For apps with [required versions](#required-versions), Furnish sends `reportVersions` (`latest` or `all`) when examining available versions. Return `latestVersion`, `versionScheme`, and `via`, and set `supportsVersionPin: true` if the tool can install an exact version. For such a manager, return `availableVersions` when asked for all. If the request includes `targetVersion`, install that exact version or return an error.

`furnish check` validates manager declarations and their script files without running them. Only the owning preset collection can use its scripts. A system install runs from the service's copy of that collection. A per-user install runs under that user's account. The [Alpine example](https://github.com/furnish-self-service/furnish/blob/main/docs/examples/apk.sh) shows the complete script.

### Furnish/Installer

Installs a `.deb`, `.rpm`, `.pkg`, `.dmg`, `.msi` or `.exe` from the collection or a vendor download. To ship the installer in the collection, set `file` to its path. Git LFS works for large files.

```yaml
- name: Company VPN
  type: Furnish/Installer
  properties:
    file: installers/vpn.msi
```

`file` is a path inside the collection. furnish refuses a path that leads outside it, also through a symbolic link, and the resource refuses to install a symbolic link. `versions` lists the versions that are acceptable (see [Required versions](#required-versions)).

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

### Furnish/NixOS

Adds packages and options to the computer's NixOS configuration and rebuilds the system, through the furnish service, or to the person's Home Manager configuration and activates it. The configuration has to be a flake set up for it (see [NixOS](admins.md#installing-apps-into-the-configuration) and [Home Manager](admins.md#installing-apps-into-home-manager)); otherwise the app shows "Needs ...". Only the furnish service changes the NixOS configuration, and it installs only the apps of the collections the administrator presets. An app from another collection can go only into a standalone Home Manager configuration.

```yaml
- name: htop
  type: Furnish/NixOS
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    name: htop
    packages: [htop]
    options:
      programs.htop.enable: true
```

| Property | Meaning |
|---|---|
| `name` | The app's name in furnish's list on the computer; unique among your apps (letters, digits, `_` and `-`) |
| `packages` | Attribute names in the configuration's nixpkgs, such as `htop` or `kdePackages.kdenlive`, added to `environment.systemPackages` or `home.packages` |
| `options` | NixOS options by name and value. Each needs an entry in the computer's `services.furnish.nix.allowedOptions` |
| `homeOptions` | Home Manager options by name and value, used instead of `options` when the app goes into Home Manager. Each needs an entry in `services.furnish.nix.allowedOptions` of the Home Manager configuration |
| `onlyIfSetUp` | `true`: the step applies only where the configuration it goes into is set up for furnish. Elsewhere furnish skips it, as for a false `condition` |

Whether the app goes into the NixOS configuration or Home Manager is up to the computer: Home Manager when furnish's Home Manager module is set up for the person, unless the setting `osSpecific.nixos.installInto` says otherwise. `packages` work in both. An app that sets options should give `options`, `homeOptions` or both. An app with options for only one of them goes there, if that is set up.

Each install or uninstall rebuilds the system or the Home Manager generation once per `Furnish/NixOS` resource, so keep an app's packages and options in one.

Without `onlyIfSetUp`, an app whose configuration isn't set up shows "Needs ...", also on Windows and macOS. With it, the step is skipped there, including on computers without NixOS, and the rest of the app works as usual. The preview shows the step as not needed and says why. The guard also keeps an existing app entry available for removal when that configuration no longer accepts new apps. Use the host setup fields to select a complementary fallback.

For example, this app adds a directory to the person's login shell path. Allow `home.sessionPath` in the Home Manager module's `services.furnish.nix.allowedOptions`. On other Unix systems the person's shell must read `~/.profile` for the fallback to take effect. If Home Manager owns that file, configure Furnish's module or choose a writable shell startup file that the person's shell sources.

```yaml
metadata:
  furnish:
    description: Add the user's local executables to the login path
    categories: [Shell]
    perUser: true
resources:
  - name: Set the login path through Home Manager
    type: Furnish/NixOS
    properties:
      name: local-bin
      onlyIfSetUp: true
      homeOptions:
        home.sessionPath: ["$HOME/.local/bin"]
  - name: Set the login path in the shell profile
    type: Furnish/Line
    condition: "[and(not(equals(context().os.family, 'Windows')), not(variables('furnish.host').nixSetup.home))]"
    properties:
      path: ~/.profile
      line: 'export PATH="$HOME/.local/bin:$PATH"'
```

A guarded NixOS step can also help another step where Home Manager manages a file. Where Home Manager writes `~/.ssh/config` (`programs.ssh`), the file is a link into the Nix store, and `Furnish/Line` can't change it. The first step has Home Manager write the `Include`. Home Manager puts all includes into one `Include` line before the first `Host`, so the `Line` step accepts an `Include` that names the file among others. After the generation is activated, the `Line` step finds that line and changes nothing. Everywhere else, only the `Line` step runs. Set [`perUser: true`](#installing-for-one-person) on the app so the SSH file belongs to the person installing it:

```yaml
- name: SSH include through Home Manager
  type: Furnish/NixOS
  properties:
    name: acme-ssh
    onlyIfSetUp: true
    homeOptions:
      programs.ssh.includes: [config.d/acme.conf]
- name: SSH include
  type: Furnish/Line
  properties:
    path: ~/.ssh/config
    line: Include config.d/acme.conf
    before: '^\s*(Host|Match)\s'
    accept: '^\s*Include\s(.*\s)?config\.d/acme\.conf(\s|$)'
```

`programs.ssh.includes` takes effect only if the person's configuration sets `programs.ssh.enable = true`. Otherwise Home Manager doesn't write `~/.ssh/config`, and the `Line` step edits the file itself. Home Manager's `allowedOptions` needs `programs.ssh.includes`.

On uninstall, a `Line` with `accept` leaves an exact match in a file linked by Home Manager to its generation in the Nix store. The `Furnish/NixOS` step removes the option. With a switch the activated generation updates the file, and with a rebuild for boot the active file stays as it is until activation. In an ordinary SSH config, the `Line` step removes the exact include and keeps other lines.

### Required versions

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
3. otherwise nothing: the app shows what it needs and what is offered, for example "Needs 128.0.3 or later; winget offers 128.0.1", and [reports](admins.md#reports) record it as a failure.

While an acceptable version is installed, furnish doesn't update it to one that isn't. With `<129`, Firefox 129 waits until the collection accepts it.

For `Furnish/Installer`, the version offered comes from the installer. For files in the collection, `furnish check` reads the version of `.deb` files (with `dpkg-deb`) and of `.rpm` files, reports an error for an unacceptable version, and warns when it cannot read the version. For a URL, it checks the pin and URL offline. During installation furnish evaluates version requirements against the verified download wherever the installer format exposes its version.

furnish never falls back to an earlier version of the app that the current `versions` don't accept (see [When an install fails](using.md#when-an-install-fails)).

Sometimes furnish can't read the installed version: for a package installed outside furnish (see `command`), or one the package manager installed without a version, such as some portable winget packages. furnish then leaves the app as it is and installs nothing over it. The app's page says that furnish can't check its version against `versions`, and where it is installed if it was installed outside furnish. `furnish list` says "version unknown", and [reports](admins.md#reports) record it.

`Furnish/NixOS` takes no `versions`: its packages come from the system's nixpkgs, which decides their version.

### Configuration

Six resources set up configuration: folders, files, parts of files and settings that a tool applies itself. They work the same on Windows, macOS and Linux, so one step covers every system. Each of them can remove what it set up, so an app made of them needs no removal steps.

Paths are absolute or start with `~/`, which resolves to the home folder of the account running the step. For the personal files in the examples below, set [`perUser: true`](#installing-for-one-person) on the app. Missing parent folders are created. A path that is a symbolic link is written through to its target. If the target can't be written, as with a file that Home Manager links from the Nix store, the step fails and names the file.

The resources that write files have these properties in common:

| Property | Meaning |
|---|---|
| `_exist` | `false` removes the file, or the resource's part of it |
| `requiresClosed` | Names of application executables that must be closed before this file changes |
| `backup` | `true` keeps a copy of the file before each change, named `<file>.<date and time in UTC>~` |
| `validate` | A command that checks the new file before it replaces the old one, with `'%s'` for its path, for example `[ssh, -G, -F, '%s', git.acme.example]`. If it exits with another code than 0, the step fails and the file stays as it was |
| `mode` | The file's mode on macOS and Linux, for example `'640'` |
| `private` | `true`: only the user may open the file. On Windows, its access list grants only the user access and inherits nothing; on macOS and Linux, its mode is 600 |

Some desktop apps keep their settings in memory and save them again on exit. Use `requiresClosed` on a `Furnish/File`, `Furnish/Line`, `Furnish/Block`, or `Furnish/Value` step that edits one of these files. For example, a Linux collection for an editor whose executable is named `notes-desktop` could include this step:

```yaml
- name: Disable the editor’s update reminders
  type: Furnish/Value
  properties:
    path: ~/.config/notes-desktop/settings.json
    format: json
    location: $.showUpdateReminders
    value: false
    requiresClosed: [notes-desktop]
```

Use executable names, such as `notes-desktop`, for processes visible to the account applying the change. The Furnish service can see applications in other users’ sessions. The desktop checks the pending operation before starting any steps and keeps it in Activity until the user closes the named applications and chooses **Recheck and continue**. The file resource checks again immediately before writing, including when invoked from the CLI. For an app in the system tray, use its Quit action. A step whose settings already match can finish while the app stays open.

On Linux, some packages start a shared `electron` or `electron<major>` executable. When its launch argument is an absolute path ending in `<app>/app.asar` or `<app>/resources/app.asar`, use the app directory name in `requiresClosed`. For example, `requiresClosed: [bitwarden]` covers both `/usr/lib/bitwarden/app.asar` on Arch and `…/opt/Bitwarden/resources/app.asar` on NixOS. Furnish ignores ASCII letter case for this directory name. Direct Linux executable names remain case-sensitive. Detection requires a readable launch argument and supports the first positional path or `--app=<path>`. An unrecognized standalone switch before that path leaves the package identity unknown. When Electron rewrites the visible command line as one title, Furnish can recognize package paths without spaces.

`mode` and `private` are part of the desired state: a file that someone made readable for others shows as an available update, and installing again restores them. A step without them leaves the mode as it is, so for a file that several `Furnish/Value`, `Furnish/Line` or `Furnish/Block` steps edit, `private: true` on one of them is enough. Steps that give the same file different modes would undo each other, so furnish refuses such an app when it loads it. Uninstalling never changes the mode, and these three resources never delete the file: it may hold the user's own settings.

A file that the account checking the app's status may not read, such as a root-only file that the furnish service wrote, counts as set up if it exists: furnish can't compare its contents as that account. The steps preview says so.

A file is replaced in one step, so programs never read half of it. It keeps its mode, owner and encoding, including a byte order mark and Windows line ends.

To tell whether an app is installed, each resource computes the change it would make to the file and reports the desired state when there is none. Installing makes that change. A setting someone changed by hand shows as an available update, and installing again sets it back.

A value that starts with `[` is a DSC expression. Write `[[` for a pattern or text that starts with `[`.

#### Furnish/Directory

A folder. `_exist: false` removes it only when it is empty. `mode` and `private` work as for files, with mode 700 for a private folder.

| Property | Meaning |
|---|---|
| `path` | The folder |

```yaml
- name: VPN folder
  type: Furnish/Directory
  properties:
    path: ~/.config/acme-vpn
    private: true
```

#### Furnish/File

A whole file, with the text in `content` or a copy of `source`, a file in the collection. furnish checks `source` like the `file` of `Furnish/Installer`: it must be in the collection, and furnish downloads it when the app is installed. Without either, the file only has to exist.

```yaml
- name: VPN profile
  type: Furnish/File
  properties:
    path: ~/.config/acme-vpn/acme.ovpn
    source: files/acme.ovpn
    private: true
```

#### Furnish/Block

Lines between two marker comments that name the app, for example `# BEGIN furnish acme/ssh` and `# END furnish acme/ssh`. If the file has the markers, the step replaces what is between them. Otherwise it adds the block at `insert`: `top`, `bottom` (the default), `before: <pattern>` or `after: <pattern>`. The position counts only for that first time, so the block stays where it is, also when someone adds lines above it. Two apps get two blocks. `_exist: false` removes the block with its markers.

| Property | Meaning |
|---|---|
| `path` | The file |
| `content` | The lines between the markers |
| `comment` | What starts a comment in the file, default `#` |
| `insert` | Where the block goes the first time |

```yaml
- name: Git server host keys
  type: Furnish/Block
  properties:
    path: ~/.ssh/known_hosts
    content: |
      git.acme.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq7…
```

#### Furnish/Line

One line, for files where marker comments aren't allowed. Patterns are regular expressions ([syntax](https://docs.rs/regex/latest/regex/#syntax)) and match anywhere in a line unless they use `^` and `$`.

| Property | Meaning |
|---|---|
| `path` | The file |
| `line` | The line |
| `match` | A pattern for the line to replace |
| `before` | The line must come before the first line that matches this pattern |
| `after` | The line must come after the last line that matches this pattern |
| `accept` | A pattern for a line that counts as the line, where the line may be. Removal leaves such a line |

Without `match`, a line equal to `line` must exist. The step adds it only if it's missing, so it never adds a second one, and removal deletes every equal line. With `match`, a line that matches must exist and equal `line`: the step replaces the first match, deletes further matches, and adds `line` if nothing matches. `line` must match `match`, or the next install wouldn't find it and would add it again; furnish refuses such an app when it loads it.

With `accept`, a line that matches it counts as present, and the step adds `line` only if no such line is in place. This suits a line that a tool may have written in another form, such as an `Include` that names other files too. `line` must match `accept`, and an app can't use both `match` and `accept`.

`before` and `after` are part of the desired state. A line that someone moved past one of these lines shows as an available update, and installing again moves it back. A missing line goes right after the `after` match, else right before the `before` match, else at the end. ssh reads an `Include` only before the first `Host` or `Match` line:

```yaml
- name: SSH configuration
  type: Furnish/Line
  properties:
    path: ~/.ssh/config
    line: Include config.d/acme.conf
    before: '^\s*(Host|Match)\s'
```

#### Furnish/Value

One value in a JSON, JSONC (JSON with comments), YAML or TOML file. The rest of the file keeps its keys, their order and its comments. A file that doesn't parse stops the step with an error, and furnish leaves it as it is.

| Property | Meaning |
|---|---|
| `path` | The file |
| `format` | `json`, `jsonc`, `yaml` or `toml`; by default from the file's extension |
| `location` | Where the value is: a JSONPath that names one place, such as `$.hosts['git.acme.example'].git_protocol`, or a JSON Pointer such as `/hosts/git.acme.example/git_protocol` |
| `value` | The value |
| `merge` | `replace` (the default), `deep` or `member` |
| `removeIf` | Instead of `value` and `merge`: a list of values. Installing leaves the value at the location as it is, and uninstalling removes it only if it is one of these |

Missing objects on the way to the location are created. Only the value at the location counts for the app's state. `[?@.name=='acme-prod']` selects the element of a list whose `name` is `acme-prod`: the step replaces that element, or adds it at the end if there is none, and `value` must have that `name`. In an object, as some tools write their entries keyed by an ID, the filter selects the member whose value has that `name`: the step replaces that member under its key, and fails if there is none, since a new member needs a key and a filter doesn't give one. To add a member, name its key in `location`. `merge: deep` sets the keys of `value` and keeps the other keys at the location. `merge: member` treats the location as a list that must contain `value` and keeps its other elements. `_exist: false` removes the value at the location, the keys of a deep merge, or the element, the whole element also with `merge: deep`.

A kubeconfig merge takes one step each for the cluster, the user and the context, and leaves the other entries as they are. kubeconfig holds credentials, and tools such as helm warn about one that others can read, so the first step has `private: true`. `merge: deep` keeps what others add to an entry: kubectl writes defaults into the user when it saves the file, such as `env: null`, and people set a context's `namespace`. With `replace`, either would show as a change, and installing would undo it.

Installing leaves `current-context` alone, so the user's choice stays. If it names the app's context when the app is uninstalled, kubectl fails on every command that doesn't name a context, since that context is gone. A step with `removeIf` removes it in that case and keeps any other value. It has nothing to do when the app is installed, so it never makes the app show as not installed or as having an update, and an uninstall that keeps the value says why in the steps preview. In [removal steps](#removal-steps), such a step needs `_exist: false`.

The whole app:

```yaml
metadata:
  furnish:
    name: Acme Kubernetes
    description: Access to the Acme production cluster with kubectl.
    categories: [Development]
resources:
  - name: Acme cluster
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.clusters[?@.name=='acme-prod']"
      private: true
      merge: deep
      value:
        name: acme-prod
        cluster:
          server: https://k8s.acme.example
  - name: Acme user
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.users[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        user:
          exec: { apiVersion: client.authentication.k8s.io/v1beta1, command: kubelogin, args: [get-token] }
  - name: Acme context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.contexts[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        context: { cluster: acme-prod, user: acme-prod }
  - name: Current context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: $.current-context
      removeIf: [acme-prod]
```

#### Furnish/Command

A setting that a tool must apply itself. `test` runs a command and checks its result: the exit code (`exitCode`, default 0), its output (`output`, or `file` for a file's content, see below), or a place in its JSON output (`jsonPath`, and `value` to compare with). `set` runs only when the test fails. `remove`, if given, runs on uninstall whatever the test says, since a failing test can also mean a setting that is there but outdated. So `remove` must succeed when there is nothing to remove. `remove: keep` leaves the setting in place on uninstall (see below). Without `remove`, furnish doesn't offer to uninstall the app. Commands are lists of the program and its arguments, run without a shell. A program that isn't installed makes the app show "Needs" with its name.

```yaml
- name: Vault server
  type: Furnish/Command
  properties:
    test:
      command: [bw, config, server]
      output: https://vault.acme.example
    set: [bw, config, server, https://vault.acme.example]
    remove: [bw, config, server, https://vault.bitwarden.com]
```

`jsonPath` takes the same paths as the `location` of `Furnish/Value`. A filter can select several values, from the elements of a list or the members of an object, and the test passes if one of them equals `value`. openvpn3 lists its profiles keyed by their D-Bus path, for example, and this test finds the profile named Acme:

```yaml
- name: VPN profile imported
  type: Furnish/Command
  properties:
    test:
      command: [openvpn3, configs-list, --json]
      jsonPath: "$[?@.name=='Acme'].name"
      value: Acme
    set: [openvpn3, config-import, --config, /etc/acme/acme.ovpn, --name, Acme, --persistent]
```

An argument that starts with `~/`, or is just `~`, stands for the home folder, the same way as in paths, so one step covers every system. A `~` elsewhere in an argument, as in `--config=~/x`, is passed on as it is. For an argument that must start with `~/` itself, write `~~/`; furnish passes it on with one `~`. The steps preview shows the arguments with `~/`:

```yaml
- name: VPN connection
  type: Furnish/Command
  properties:
    test: { command: [acme-vpn, list], output: Acme }
    set: [acme-vpn, import, ~/.config/acme/acme.ovpn]
```

Some steps set up something that other apps or programs use too, which uninstalling must not take away. With `remove: keep`, uninstalling leaves the setting as it is, the app can still be uninstalled, and the uninstall preview says the step is kept. A plugin app for a coding assistant is an example: uninstalling removes the plugin and keeps the vendor's marketplace, which other plugins come from:

```yaml
- name: Acme marketplace
  type: Furnish/Command
  properties:
    test:
      command: [assistant, plugin, marketplace, list, --json]
      jsonPath: "$[?@.name=='acme-marketplace'].url"
      value: https://git.acme.example/marketplace.git
    set: [assistant, plugin, marketplace, add, https://git.acme.example/marketplace.git]
    remove: keep
- name: Acme plugin
  type: Furnish/Command
  properties:
    test: { command: [assistant, plugin, list, --json], jsonPath: "$[?@.id=='tools@acme-marketplace'].id", value: tools@acme-marketplace }
    set: [assistant, plugin, install, tools@acme-marketplace]
    remove: [assistant, plugin, uninstall, tools@acme-marketplace]
```

An app whose steps are all kept still tests as installed after uninstalling, so uninstalling it fails. Give such an app at least one step that uninstalling removes.

Many tools keep their own copy of a file the app writes, and the test is whether that copy still matches the file. `file` compares the output with the file's content instead of a fixed text. The path is absolute or starts with `~/`. `ignoreLines`, a regular expression, leaves out the lines it matches, in the output and in the file (or in `output`). furnish also ignores Windows line ends, spaces at the end of a line, and blank lines at the start and end, so this works the same on Windows, macOS and Linux, without a shell. The steps preview names the command and the file, and if they differ, it shows the differing lines as for a file change, or only how many if only the file's owner may read it. A file that the account checking the app's status may not read counts as matching, as with the file resources.

When a tool keeps its own copy, give the Command step a collection file with `source` or inline text with `content`. A command argument written as `'%s'` receives the path of a private temporary file. The file exists only while that command runs. A source file keeps its filename and extension. For `test.file` or `present.file`, `'%s'` compares the command's output directly with the supplied input. Only the tool's own copy determines whether the step is installed.

For example, openvpn3 drops comment lines and adds blank lines when it exports a profile. This step recognizes a matching profile even if someone imported it by hand:

```yaml
- name: VPN profile in openvpn3
  type: Furnish/Command
  properties:
    source: files/acme.ovpn
    test:
      command: [openvpn3, config-dump, --config, Acme]
      file: '%s'
      ignoreLines: '^(#|$)'
    present: { command: [openvpn3, config-manage, --config, Acme, --show] }
    set: [openvpn3, config-import, --config, '%s', --name, Acme, --persistent]
```

Use `'%s'` as a whole argument in `test.command`, `present.command`, `set` or `remove`. If a shell is needed, pass the file as a positional argument to the script. The preview shows the command as written and gives only line counts for differences in the input.

When `test.file` or `present.file` names a file an earlier `Furnish/File` step will write, the preview compares the command output with the planned file contents. During installation the command checks the file on disk after the earlier step finishes.

Other tools that keep their own copy work the same way, for example `nmcli connection export`, `kubectl config view --minify` and `security find-certificate -p`.

A failing test can mean that the setting is missing or that it is there but outdated. `present`, with the same properties as `test`, tells the two apart: if `test` fails and `present` passes, the app shows an update instead of "not installed", and it can be uninstalled. Without `present`, a failing test counts as missing.

A VPN connection that NetworkManager imports from a profile is an example. When the collection brings a new profile, the connection is still there, but it was imported from the old one:

```yaml
- name: VPN profile
  type: Furnish/File
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    path: /etc/openvpn/acme-vpn.ovpn
    source: files/acme.ovpn
- name: VPN connection
  type: Furnish/Command
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    # Imported from this profile: the copy made at import time is the same.
    test: { command: [cmp, -s, /etc/openvpn/acme-vpn.ovpn, /var/lib/acme-vpn/imported.ovpn] }
    # The connection exists, from whichever profile.
    present: { command: [nmcli, connection, show, id, Acme VPN] }
    set: [sh, -c, "nmcli connection delete id 'Acme VPN'; nmcli connection import type openvpn file /etc/openvpn/acme-vpn.ovpn && nmcli connection modify acme-vpn connection.id 'Acme VPN' && mkdir -p /var/lib/acme-vpn && cp /etc/openvpn/acme-vpn.ovpn /var/lib/acme-vpn/imported.ovpn"]
    remove: [sh, -c, "nmcli connection delete id 'Acme VPN'; rm -f /var/lib/acme-vpn/imported.ovpn"]
```

### Other DSC resources

An app can also use other DSC resources. An app that sets up a VPN connection or a setting needs no package at all.

```yaml
- name: Acme proxy
  type: Microsoft.Windows/Registry
  condition: "[equals(context().os.family, 'Windows')]"
  properties:
    keyPath: HKCU\Software\Acme
    valueName: Proxy
    valueData: { String: proxy.acme.example }
```

furnish includes the resources of DSC 3.3.0, so computers don't need DSC installed for them. Among them:

| System | Resources |
|---|---|
| Windows | `Microsoft.Windows/Registry`, `Microsoft.Windows/EnvironmentVariable`, `Microsoft.Windows/Service`, `Microsoft.Windows/FirewallRuleList`, `Microsoft.Windows/WindowsFeatureList`, `Microsoft.Windows/OptionalFeatureList`, `Microsoft.DSC.Transitional/WindowsPowerShellScript`, `Microsoft.Windows/WindowsPowerShell` (Windows PowerShell DSC resources) |
| All systems | `Microsoft.Filesystem.File/Content`, `Microsoft.DSC.Transitional/RunCommandOnSet`, `Microsoft.DSC/Group`, `Microsoft.DSC/Include`, `Microsoft.DSC/Assertion` |
| Where PowerShell 7 is installed | `Microsoft.DSC.Transitional/PowerShellScript`, `Microsoft.DSC/PowerShell` (PowerShell DSC resources) |

Microsoft's [resource reference](https://learn.microsoft.com/powershell/dsc/reference/resources/builtin?view=dsc-3.0) describes their properties.

furnish also finds resources where DSC does: in the folders on `PATH` and where DSC is installed. A resource the computer doesn't have makes the app show "Needs the DSC resource ...". So install the resource on the computers that need it, for example as a dependency. A resource whose `condition` is false on a computer doesn't need to be there. A folder elsewhere that declares a type furnish includes, such as an installed DSC of another version, isn't searched for resources: furnish's version is the one apps get.

furnish uses `test` to tell whether an app is installed. A resource that is not in the desired state and doesn't report `_exist` counts as not installed. Some resources always report the desired state, such as `RunCommandOnSet`. Combine those with a resource that can tell. The same test runs right after an install, so a resource whose `test` doesn't match what its `set` does makes every install fail.

For `--dry-run` and the steps preview, furnish's own resources describe what they would do. The package and installer resources show their exact commands. The configuration resources name each file and what happens to it, with the number of lines added and removed. `Furnish/Line` also names the line, and `Furnish/Command` shows its test command with the output it expected and got, and whether `present` found an outdated setting. Other resources show their type and properties.

`Furnish/File`, `Furnish/Block`, `Furnish/Line` and `Furnish/Value` also show the file's changed lines, with 3 lines around each change. For `Furnish/Value`, that is the text as furnish writes it, so you see which comments and formatting stay. When its lines are hidden (see below), it shows the value at its location before and after instead. A file that ends without a line break shows a note after its last line, as `git diff` does, instead of a changed line. In the app, the lines are collapsed under the step; `--dry-run` prints them as a unified diff. When someone changed the file by hand, they show what installing again changes back. A step shows at most 200 lines and cuts lines after 200 characters. A file that isn't text shows "binary file, not shown".

The changed lines of a file that may hold secrets aren't shown, only their number. That is a file with `private: true`, a file with a `mode` that doesn't let group and others read it (such as `600`), and a file that already has such a mode or, on Windows, an access list for the user only. A file the user can't read at all (only an administrator may) shows neither.

No other resource may declare one of furnish's types, such as `Furnish/Package`. furnish refuses to run when DSC finds one, whatever its version.

Your organization can limit which resource types apps may use (see [For admins](admins.md#the-policy)).

### Settings available to resources

furnish passes its settings to every resource it runs as environment variables, so your own resources and commands can use them. `FURNISH_CONFIG__` variables hold every setting with the value in effect, as `furnish settings` shows it. `FURNISH_DETECTED__` variables hold what furnish found out, such as where the NixOS configuration is.

The name is the setting's path in `settings.json`. Levels are separated by `__`, and the words of a name by `_`, in capitals:

| Setting | Variable |
|---|---|
| `updateOnLaunch` | `FURNISH_CONFIG__UPDATE_ON_LAUNCH` |
| `osSpecific.nixos.flakeAttribute` | `FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE` |

Texts are passed as they are, `true` and `false` as words, numbers as digits, and lists and maps as JSON. A value nobody set and furnish doesn't know is empty. Option values aren't passed: an app gets its own as parameters (see [Questions and options](#questions-and-options)).

On NixOS, furnish passes where the configuration is, whether a setting or a search found it:

| Variable | Value |
|---|---|
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION` | the configuration's folder |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_KIND` | `flake` or `channels` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_SOURCE` | `module` (furnish's NixOS module), `policy`, `setting`, `etc-nixos`, `nh` or `nix-path` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_OWNER` | the folder's owner, as `<uid>:<user>` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_OTHERS_CAN_WRITE` | `true` if its group or everyone can write to it |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE` | the name in `nixosConfigurations`, by default the host name |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__HOME_MANAGER_CONFIGURATION` | the Home Manager configuration's folder |

```yaml
- name: Show the configuration
  type: Microsoft.DSC.Transitional/RunCommandOnSet
  properties:
    executable: sh
    arguments: ["-c", "echo \"$FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION\""]
```

In the furnish service, resources run as root, and the settings come from the admin policy and furnish's defaults only, never from a person's own settings.

### Removal steps

Many resources can't remove what they set up. `File/Content` writes a file but doesn't delete it. `RunCommandOnSet` and the PowerShell script resources run their command or script every time. For such an app, list the steps that remove it under `metadata.furnish.uninstall`. They are DSC resources in the same format as `resources`, with conditions and `dependsOn`:

```yaml
metadata:
  furnish:
    uninstall:
      - name: delete the profile
        type: Microsoft.DSC.Transitional/RunCommandOnSet
        condition: "[equals(context().os.family, 'Linux')]"
        properties:
          executable: rm
          arguments: [-f, /etc/acme/profile.conf]
resources:
  - name: profile
    type: Microsoft.Filesystem.File/Content
    condition: "[equals(context().os.family, 'Linux')]"
    properties:
      path: /etc/acme/profile.conf
      content: "server = vpn.acme.example"
```

To uninstall the app, furnish runs the removal steps in their order instead of the resources. The steps preview and `furnish uninstall --dry-run` show them. A removal step whose command exits with a code other than the expected one fails the uninstall. Afterwards furnish tests the app's resources as usual. If they still find the app, the uninstall fails with a message that says so. The organization's list of allowed resource types and the checks on installer files apply to removal steps too.

Without removal steps, furnish looks at each resource that applies on the computer. A resource can remove if its manifest has a `delete` operation or its `set` handles `_exist` (`handlesExist: true`). furnish's own resources and `Microsoft.Windows/Registry` can, except a `Furnish/Command` without `remove`. If one of the app's resources can't, furnish doesn't offer to uninstall the app: the app's page shows a note instead of the Uninstall button, and `furnish uninstall` refuses. `furnish check` warns about such apps.

### Dependencies

An app can depend on other apps of its collection. furnish installs required dependencies first. Optional ones are preselected, and people can deselect them when they install the app.

```yaml
metadata:
  furnish:
    dependencies:
      - app: homebrew
        required: true
        condition: "[equals(context().os.family, 'macOS')]"
      - app: fonts
```

A dependency can have a DSC `condition`, whether it is required or optional. Furnish includes the dependency when the expression is true and skips it when false. In this example, only macOS requires Homebrew, and fonts are optional on every system. Conditions can also use the app’s parameters and the [host information](#host-information) available to resource steps.

If a resource needs a tool that is missing, such as Homebrew on a Mac, the app shows "Needs Homebrew" instead of an Install button. A dependency can install the tool.

## Icons

Apps set `metadata.furnish.icon`, kits and `collection.yaml` set `icon`: the path of an SVG, PNG, JPEG or WebP file in the repository. The collection's icon appears next to its name on the Collections page, on app cards and on app pages.

The app shows icons on its light or dark background as they are. For a logo that is hard to see in dark mode, such as dark lines without a background of their own, `iconDark` (`metadata.furnish.iconDark` for apps) names the image to show instead in dark mode. Many projects publish a version of their logo for dark backgrounds that fits:

```yaml
metadata:
  furnish:
    icon: icons/helm.svg
    iconDark: icons/helm-white.svg
```

## Kits

A kit installs several apps of the collection together:

```yaml
# kits/office.yaml
name: Office              # shown instead of the ID; optional
description: Office suite, email and a password manager
categories: [Office]
icon: icons/office.svg
items:
  - app: libreoffice
    required: true       # always installed with the kit
  - app: thunderbird
    required: true
  - app: keepassxc       # optional and preselected
  - app: 7zip
    selected: false      # optional, not preselected
```

![The Office kit with its four apps.](images/kit.webp)

## Questions and options

`collection.yaml` declares options that all of the collection's apps can use. An option marked `required` is a question: furnish asks it when the collection is added, and at each start until someone answers.

```yaml
icon: icons/collection.svg
parameters:
  role:
    type: string
    required: true
    label: What do you work on?
    allowedValues:
      - { value: developer, label: Software development }
      - { value: design, label: Design and media }
      - { value: office, label: Office work }
```

An option with `allowedValues` is shown as a list to choose from. An app uses an option by declaring it as a DSC parameter, then uses it in conditions and properties. furnish takes an option's value from the first of these that sets it: a locked value in the admin policy, the user's value for the app, the policy's default for the app, the user's value for the collection, the policy's default for the collection, the default in `collection.yaml`, the app's own default. A value is used as it is: text that starts with `[` is not evaluated as a DSC expression, except the app's own default.

## Recommendations

```yaml
metadata:
  furnish:
    recommended: "[equals(parameters('role'), 'office')]"   # or: true
```

Recommended apps appear first on Discover. The expression can use the collection's options and the app's own.

## Translations

A collection can translate its texts into any language. Each language has one file, `locales/<lang>.yaml`, named with a language tag such as `de`, `fr` or `pt-BR`:

```yaml
# locales/de.yaml
categories:                # a category as the apps write it, and its name
  Office: Büro
apps:
  thunderbird:             # the app's ID
    name: Thunderbird
    description: E-Mail, Kalender und Kontakte.
kits:
  office:
    name: Büro
    description: Office-Paket, E-Mail und Passwortmanager
parameters:                # options and questions of collection.yaml and of the apps
  role:
    label: Woran arbeitest du?
    description: Empfiehlt die Apps für deine Rolle.
    allowedValues: { developer: Softwareentwicklung, design: Design und Medien, office: Büroarbeit }
```

Every entry is optional. The texts in the app files, kit files and `collection.yaml` are in English unless `collection.yaml` names another language, for example `language: de`.

For each text, furnish shows the first of these that exists:

1. the translation into the app's language, the one set under Language or the system's,
2. the translation into the fallback language, English unless people or the policy choose another,
3. the text as written in the collection.

A translation file for `pt` also serves people who use `pt-BR`, and the other way round. If the app's language or the fallback language is the collection's own `language`, furnish shows the collection's texts. A collection written in German with `language: de` shows its texts to German speakers, and to everyone else when it has no translation for them.

The sidebar lists each category once, even if several collections use it. If they translate it differently, the sidebar uses the translation of the first collection in alphabetical order. Search looks at the texts in the app's language, in the fallback language and as written in the collection, so an English word finds an app on a German screen. Reports and logs use IDs, and so do commands such as `furnish install`. `furnish list` shows the IDs with the translated texts.

## Check a collection

`furnish check` reads every file of a collection the way furnish does and installs nothing. It reports what would break for the people installing from it: files that don't load, including translations, installers and icons that aren't in the repository, dependencies and kit items that don't exist, conditions and `recommended` expressions that don't evaluate, and apps that have a resource for no system. It also checks `versions`: an entry that doesn't parse is an error, a range no version can match is a warning, and so is an installer file whose version it can't read; an installer file that `versions` doesn't accept is an error. A resource type that DSC doesn't find on the computer running the check is a warning, since it may be installed where the app runs. An app without removal steps whose resources can't remove what they set up is a warning too. Only types DSC finds on the computer running the check are judged. For each app it shows which resources run on Windows, macOS and Linux. For each translation it warns about apps, kits, categories and options the collection doesn't have, and counts what is translated. Missing translations are not errors. It exits with 1 when there are errors.

```
$ furnish check
✓ collection.yaml  options
✓ gimp             app  Windows: gimp (winget) · macOS: gimp · Linux: gimp
✗ vpn              app  Windows: setup · macOS: – · Linux: –
    error: setup: installers/vpn.msi is not in the collection
! slack            app  Windows: slack (winget) · macOS: slack (Homebrew) · Linux: –
    warning: no categories (metadata.furnish.categories)
✓ office           kit
✓ locales/de.yaml  translation

Translations (texts without one show in the fallback language or the collection's own):
  de: 3 of 3 apps, 1 of 1 kit, 2 of 3 categories, 1 of 1 option

3 apps, 1 kit: 1 error, 1 warning
```

To check every change before it reaches people, run it in the repository's CI. On GitHub:

```yaml
# .github/workflows/furnish.yml
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: furnish-self-service/furnish/check@main
```

On GitLab:

```yaml
# .gitlab-ci.yml
furnish-check:
  image: alpine:3.24
  script:
    - apk add --no-cache curl git
    - curl -fsSL https://furnish-self-service.github.io/furnish/alpha/furnish-linux-x86_64 -o /usr/local/bin/furnish
    - chmod +x /usr/local/bin/furnish
    - furnish check
```

The preview evaluates `context().os.family` for each system. It marks a step `(depends on host)` when its result depends on the target computer's distribution, available package managers, or configured Nix destinations. Reads of other `context()` fields also depend on the target computer.

### Check target profiles in CI

Use `furnish check . --profiles targets.yaml` to check the same collection for specific computers. Each named profile supplies the OS family (`Linux`, `macOS`, or `Windows`) and the available package managers. Linux profiles can also supply the distribution ID and the ordered `ID_LIKE` list from `os-release`. List the native managers available on the target in Furnish’s priority order. On Unix the order is `pacman`, `apt`, `dnf`, `zypper`, `brew`, `nix`, then `mise`. On Windows list `winget` before `mise`. Include a collection manager's ID when its script and prerequisites are available on that target.

For example, an admin can verify which package implements a step named `Editor` and which dependency applies to developers:

```yaml
profiles:
  developer-laptop:
    family: Linux
    distribution: cachyos
    distributionLike: [arch]
    packageManagers: [pacman, nix]
    options: { role: developer }   # collection.yaml parameters
    appOptions:
      editor: { plugins: true }    # DSC parameters declared by this app
    expect:
      editor:
        dependencies: [developer-tools]
        steps: [Editor]
        skipped: ["Install on Windows"]
        packages:
          Editor: { manager: pacman, name: code }
```

The report lists each active dependency, the install steps in execution order, the package selection and the condition behind each skip. Active dependencies include optional dependencies users can select. An applicable package step lists a missing prerequisite when its manager is absent from the profile. A preceding dependency may supply it during installation. Omit `expect` to inspect the decisions first. Each field you include checks the entire list or package map for that app. An empty list asserts that no entries apply. A mismatch prints the expected and actual values and exits with status 1. Defaults come from the collection and app documents. Supply option values for the choices you want to exercise.

The family preview reports a dependency cycle as an error when its conditions hold together on one system. A cycle that depends on unresolved host facts produces a warning. Profiles check the graph again with their concrete facts and options. Profile assertions cover install-time selection. A condition or package expression that needs a live machine fact outside the profile, such as an environment variable, produces a diagnostic.

A profile assumes a fresh target with no configured Furnish NixOS or Home Manager module. A `Furnish/NixOS` step with `onlyIfSetUp: true` appears in `skipped` by default, including when `nix` is present in `packageManagers`. To test a machine with a configured destination, include `nix` in `packageManagers` and declare the ready destinations:

```yaml
profiles:
  mac-home:
    family: macOS
    packageManagers: [brew, nix]
    nixSetup: { home: true }
  nixos-workstation:
    family: Linux
    packageManagers: [nix]
    nixSetup: { system: true, home: true }
```

Use `home` for Home Manager on Linux or macOS and `system` for the NixOS system configuration on Linux. These values also supply `variables('furnish.host').nixSetup.home` and `.nixSetup.system` in target expressions. Each defaults to `false`. Steps declaring only `homeOptions` require the home destination. Steps declaring only `options` require the system destination. For packages alone or options for both destinations, the profile uses home when configured and system otherwise. Windows profiles always skip guarded steps. A step without the guard keeps its normal condition.

The [two-profile example](https://github.com/furnish-self-service/furnish/blob/main/docs/examples/target-profiles.yaml) runs against Furnish's test collection:

```sh
furnish check tests/fixtures/collection --profiles docs/examples/target-profiles.yaml
```

In the GitHub Action, pass the file with `with: { profiles: targets.yaml }`.

## Private repositories

A collection can be a private repository:

- Over HTTPS, furnish asks for a username and a password or access token when the server requires one, keeps them in the system's credential store, and sends them to that repository only. On the command line: `furnish collection add <url> --username alice`, which asks for the password, or `--password-stdin` for scripts.
- Over SSH, furnish uses git as it is set up: your keys, `ssh-agent` and `~/.ssh/config`. Connect to the server once with `ssh` to accept its host key.

git's own credential helpers and credentials in the URL also work.

![The add form with the login fields for a private HTTPS repository.](images/private-collection.webp)
