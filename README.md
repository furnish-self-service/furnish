<img src="logo.svg" alt="furnish logo: a white armchair on a teal tile" width="96" align="right">

# furnish

A self-service app catalog for Linux, macOS and Windows. Your organization lists apps in git, and users install them with one command.

**Status:** early prototype. The CLI, the desktop app and the package resource work on Linux, macOS and Windows. Apps run with administrator rights, so only add collections you trust.

## What you get

Admins publish apps in a git repository, called a *collection*. Each app is one [DSC v3](https://learn.microsoft.com/powershell/dsc/overview) configuration file. Users manage those apps with these commands:

| Command | What it does |
|---|---|
| `furnish collection add <git-url>` | Clones a collection and makes its apps available |
| `furnish list` | Shows every app with its state: installed, update available or not installed |
| `furnish install <app>` | Brings the app to the state its file describes, and does nothing if it's already there |
| `furnish uninstall <app>` | Removes what the app installed |
| `furnish collection update` | Pulls the latest app definitions |

## Why it is different

- **No server or agent.** furnish is one binary that reads collections straight from git.
- **App state comes from the definition.** furnish runs a DSC test: it compares the machine with the app's file instead of keeping its own records.
- **DSC is built in.** The [DSC](https://github.com/PowerShell/DSC) engine is compiled into furnish as a library, so users don't install it separately.
- **Admins only need YAML.** Common cases work with the bundled `Furnish/Package` resource, which uses pacman, apt, dnf, zypper or Homebrew.

## Start here

On Linux, install furnish (desktop app and CLI) from its package repository. Only the `alpha` channel exists so far: builds of the main branch, versioned `0.1.0+alpha.<build>`. The repository is signed with the key [`gpg-key.asc`](packaging/repo/gpg-key.asc) (fingerprint `3A87 437D E7A2 B5DE 9436  AD89 8C29 D9C7 FFF4 823E`).

```sh
# Debian, Ubuntu, Mint
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/furnish.asc https://furnish-self-service.github.io/furnish/gpg-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/furnish.list https://furnish-self-service.github.io/furnish/alpha/furnish.list
sudo apt update && sudo apt install furnish

# Fedora, Enterprise Linux 10 (with EPEL and CRB)
sudo curl -fsSLo /etc/yum.repos.d/furnish.repo https://furnish-self-service.github.io/furnish/alpha/furnish.repo
sudo dnf install furnish

# openSUSE
sudo zypper addrepo https://furnish-self-service.github.io/furnish/alpha/furnish.repo
sudo zypper install furnish
```

Updates then arrive with the system's own updates. To build from source instead, you need git and Rust 1.91 or later:

```sh
git clone git@github.com:furnish-self-service/furnish.git
cd furnish
cargo install --path .
furnish collection add git@github.com:furnish-self-service/self-service-example.git
furnish list
```

`furnish list` prints the catalog. Symbols show each app's state:

```
   App     Collection            Categories          Description
·  cowsay  self-service-example  Fun                 Let an ASCII cow say stuff in your terminal!
✓  htop    self-service-example  System, Monitoring  Interactive process viewer for the terminal.

✓ installed  ↑ update available  · not installed  ? unknown
```

## Desktop app

`gui/` is a desktop app for the same features: browse and search the catalog, install, update and uninstall apps, and manage collections. It uses [Tauri 2](https://tauri.app) with a Svelte frontend and calls the furnish library directly, without going through the CLI.

```sh
cd gui
npm ci
npx tauri dev -- --profile fast   # development; the fast profile avoids DSC's pwsh build step
npm run bundle                    # release build with installers in target/release/bundle/, CLI included
```

On Linux you need WebKitGTK 4.1, for example `webkit2gtk-4.1` on Arch or `libwebkit2gtk-4.1-dev` on Debian and Ubuntu. CI builds installers for Linux, macOS and Windows.

The Linux packages for distribution are built on Ubuntu 22.04 with a static `furnish` CLI, so they run on glibc 2.34 and newer with WebKitGTK 4.1. CI installs them on Debian 12, Ubuntu 22.04 and 24.04, Fedora, openSUSE Leap 15.6 and Tumbleweed, and AlmaLinux 10 (`tests/packages.sh`). Enterprise Linux 10 needs EPEL and CRB enabled for WebKitGTK 4.1; Enterprise Linux 9 has no WebKitGTK 4.1, so only the CLI and the service run there (the static `furnish` binary). To build them:

```sh
docker run --rm -v "$PWD:/src" -w /src -e HOST_UID=$(id -u) ubuntu:22.04 packaging/linux/build.sh
```

`tests/ui/run.sh` runs UI tests against the real app: Ubuntu 24.04 in Docker, Xvfb, and `tauri-driver`, with a fake package manager. Without arguments it also builds the app in that container, so it works on any Linux host with Docker and Node.

Each app's detail page has three collapsed sections for experts:

- **Steps** previews installing, updating or uninstalling, without changing anything: each step, the exact command and whether it needs administrator rights. While an operation runs, it follows those steps live: done, current and upcoming steps, with start time, duration and result.
- **Activity log** shows each DSC step and the package manager's output during install, update or uninstall.
- **DSC configuration** shows the app's file.

Installs, updates and uninstalls run one at a time. Starting another while one runs queues it: its button shows "Queued" with a ✕ to remove it, and the **Activity** page lists what runs now and what is queued.

While an install, update or uninstall runs, its button turns into a progress indicator with a stop control (✕) inside: the first click stops after the current step, the second (■) interrupts the current step. Dismissing the administrator password dialog cancels the operation instead of failing it.

The app updates collections when it starts and every 30 minutes while it runs. Both are settings on the Collections page, in `settings.json` in furnish's data directory (admins can deploy it), and in the CLI: `furnish settings --update-on-launch false --update-interval 60` (`0` turns the interval off).

### Settings and admin policy

Users' own settings live in `settings.json` in furnish's data directory (`~/.local/share/furnish` on Linux, `~/Library/Application Support/furnish` on macOS, `%APPDATA%\furnish` on Windows). It only holds what the user changed.

Admins set policy where each OS keeps managed settings, which users can't write:

| OS | Location |
|---|---|
| Linux | `/etc/furnish/policy.json` |
| macOS | Managed preferences for `io.github.furnish-self-service.furnish` (an MDM configuration profile), read from `/Library/Managed Preferences` |
| Windows | Registry value `Policy` (string, the JSON below) in `HKLM\SOFTWARE\Policies\furnish` (Group Policy, Intune) |

```json
{
  "settings": {
    "updateIntervalMinutes": { "value": 60, "locked": true },
    "loadAwareUpdates": { "value": false }
  },
  "collections": {
    "preset": [{ "name": "acme-it", "url": "git@github.com:acme/it-apps.git" }],
    "usersCanAdd": false
  },
  "parameters": {
    "acme-it": { "userInstall": { "value": true, "locked": true } }
  }
}
```

Each setting is either a default the user can change, or `locked`. `parameters` sets [options](#options) the same way, for a collection (`acme-it`) or one app (`acme-it/kubectl`). Preset collections are cloned for every user and can't be removed; with `usersCanAdd: false`, users can't add others, and collections they added before are ignored. On macOS the plist dictionary has the same structure.

### Branding

The desktop app can carry your organization's name, logo and colors instead of furnish's. Set them in the policy:

```json
{
  "branding": {
    "name": "Acme Apps",
    "logo": "acme-it/branding/logo.svg",
    "accent": "#e30613",
    "accentDark": "#ff5a4f",
    "theme": { "light": { "sidebar": "#fff5f4" }, "dark": {} }
  }
}
```

- `name` replaces "furnish" in the window title, the sidebar and the app's texts.
- `logo` is an SVG, PNG, JPEG or WebP file: an absolute path, or `<preset collection>/<path>` so it ships with your collection.
- `accent` is your brand color. The app derives its accent shades from it and keeps text in that color readable (at least 4.5:1 contrast) for both themes. `accentDark` sets the dark theme's color; it defaults to a lighter `accent`.
- `theme` overrides any other color of the app per theme, by the names in [`gui/src/app.css`](gui/src/app.css) without `--` (for example `canvas`, `sidebar`, `ink`, `highlight`). Values are `#rrggbb`.

The installed app's icon and desktop entry stay furnish's: they come from the package.

### Users without administrator rights

By default furnish runs as the user and asks for an administrator password when a step needs one, which suits small organizations where users are local admins. If users aren't admins, run the furnish service: it installs the apps of your preset collections system-wide on their behalf. Enable it with a `service` section in the policy:

```json
{
  "collections": {
    "preset": [{ "name": "acme-it", "url": "git@github.com:acme/it-apps.git" }],
    "trustedKeys": ["ssh-ed25519 AAAAC3Nz… it-team"]
  },
  "service": { "allowedGroup": "staff" }
}
```

- The service keeps its own copies of the preset collections, which users can read but not change. Collections users add still install as the user.
- Requests name an app and option values only. The service accepts only options the app declares, with the right type, never against a locked policy value, and no free-text strings (declare `allowedValues`).
- With `trustedKeys`, preset collections only move to commits signed by one of these SSH keys (`git commit -S` with `gpg.format=ssh`). Without it, whoever can push to the repository can run code as root on every computer.

Install the service on each computer as administrator, for example from your MDM or Intune:

```sh
sudo furnish service install     # Linux, macOS (Windows: furnish service install, elevated)
```

It registers furnish with the OS's service manager, pointing at that `furnish` binary, which the desktop app's installers include (`/usr/bin/furnish`, `furnish.app/Contents/MacOS/furnish`, `C:\Program Files\furnish\furnish.exe`). `furnish service uninstall` removes it.

| | Service | Collections | May install, besides root/SYSTEM |
|---|---|---|---|
| Linux | systemd unit `furnish`, `/run/furnish.sock` | `/var/lib/furnish` | members of `allowedGroup`; others what polkit allows for `io.github.furnish.install` (default: an administrator password; [rule example](packaging/linux/50-furnish.rules.example)) |
| macOS | launchd daemon `io.github.furnish-self-service.furnish`, `/var/run/furnish.sock` | `/Library/Application Support/furnish` | members of `allowedGroup`, nested and directory groups included |
| Windows | service `furnish` as SYSTEM, `\\.\pipe\furnish` | `C:\ProgramData\furnish` | elevated administrators; members of `allowedGroup` (a name or SID) |

Apps whose options make them install into the user's account (`perUser` in the app's metadata, see [Options](#options)) install as the user, not through the service; their system-wide dependencies still go through it.

On Linux desktops, `tauri dev` builds get furnish's icon only with a desktop entry: run `gui/scripts/dev-desktop-entry.sh` once (`--remove` undoes it). Installed packages include one.

The CLI has the same features: `furnish install <app> --dry-run` and `furnish uninstall <app> --dry-run` preview the steps, and `--verbose` shows the log.

## Example app definition

An app is a file in the collection's `apps/` directory. Its name, without `.dsc.yaml`, is the app's name, and it holds a standard DSC configuration. furnish reads only `metadata.furnish`:

```yaml
# apps/cowsay.dsc.yaml
metadata:
  furnish:
    description: Let an ASCII cow say stuff in your terminal!
    categories: [Fun]
resources:
  - name: cowsay package
    type: Furnish/Package
    properties:
      name: cowsay
```

### Dependencies and missing tools

An app can depend on other apps of its collection. Required dependencies are always installed first; optional ones are preselected unless `selected: false`, and users can deselect them when installing (the app asks in a dialog, the CLI takes `--with`/`--without`). The detail page lists them.

```yaml
metadata:
  furnish:
    description: Code editor with our settings
    dependencies:
      - app: homebrew          # required: installed first if missing
        required: true
      - app: node              # optional, preselected
      - app: docker            # optional, not preselected
        selected: false
resources:
  - name: vscode
    type: Furnish/Package
    properties:
      name: visual-studio-code
      packageManager: brew     # optional: use this package manager instead of the first one found
```

A package can have another name with some package managers: `pacmanName`, `aptName`, `dnfName`, `zypperName`, `brewName` and `nixName` override `name` for that one (for example `name: code`, `nixName: vscode`).

With Nix (NixOS, or Nix on another system), furnish installs into the user's profile (`nix profile install nixpkgs#<name>`), and as root, through the furnish service, into the shared profile `/nix/var/nix/profiles/furnish`, which the NixOS module puts on everyone's `PATH`. `nixpkgs` is the registry's entry, which NixOS systems built from a flake pin to the system's nixpkgs; the `nixpkgs` property names another flake reference, for example a pinned one. An update is available when that nixpkgs has another version.

If a resource needs a tool that isn't installed, such as Homebrew on a Mac, the app shows what it needs ("Needs Homebrew") instead of failing; a dependency can provide it. Resources report a missing tool by printing `furnish: needs <what>` to stderr and exiting with code 11.

### Options

Options let one app definition install in different ways, for example system-wide or just for the user, instead of shipping `kubectl` and `kubectl (user)`. An option is a [DSC parameter](https://learn.microsoft.com/powershell/dsc/concepts/configuration-functions/parameters): resources and dependencies use it in `condition`s and property expressions.

A collection declares its avatar and the options its apps share in `collection.yaml`:

```yaml
icon: icon.svg                 # the collection's avatar (SVG, PNG, JPEG or WebP in the repository)
parameters:
  role:
    type: string
    required: true             # asked when the collection is added, and on each launch until answered
    label: What do you work on?
    allowedValues:             # a dropdown; labels are optional
      - { value: developer, label: Software development }
      - { value: design, label: Design }
  userInstall:
    type: bool
    default: false
    label: Install just for me
    description: Installs into your user account; no administrator rights needed.
```

An app uses an option by declaring it in its DSC `parameters` (with its own `defaultValue`, used when the collection doesn't declare it):

```yaml
parameters:
  userInstall:
    type: bool
    defaultValue: false
metadata:
  furnish:
    perUser: "[parameters('userInstall')]"         # installs as the user, not through the furnish service
    dependencies:
      - app: homebrew
        required: true
        condition: "[parameters('userInstall')]"   # only needed for a user install
resources:
  - name: kubectl (system-wide)
    type: Furnish/Package
    condition: "[not(parameters('userInstall'))]"
    properties: { name: kubectl }
  - name: kubectl (just for you)
    type: Furnish/Package
    condition: "[parameters('userInstall')]"
    properties: { name: kubectl, packageManager: brew }
```

A required option needs an answer before the collection's apps can be installed. `furnish collection add <url> --option role=developer` answers on the command line; otherwise furnish asks: the CLI in a terminal, the desktop app in a dialog that can be postponed. That also happens on the next launch for collections the policy presets or that were added by hand. A value the admin policy sets counts as an answer.

Apps can be recommended on the Discover page, for everyone or depending on the options:

```yaml
metadata:
  furnish:
    recommended: "[equals(parameters('role'), 'developer')]"   # or: true
```

The expression can use the collection's options and the app's own; `furnish list` marks recommended apps with ★.

The value comes from, first match wins: a locked policy value (app, then collection), the user's value for the app, a policy default for the app, the user's value for the collection, a policy default for the collection, the collection's `default`, the app's `defaultValue`. Users set options on the app's page and, for all of a collection's apps, on the Collections page; in the CLI with `furnish option list|set|reset <collection or app> [name] [value]`.

furnish remembers the values an app was installed with: status checks, updates and uninstalls use them. When the current values differ, the app offers a reinstall (`furnish reinstall <app>`), which uninstalls with the old values and installs with the new ones, installing new dependencies first.

### Updates

`Furnish/Package` reports an update when the package manager knows a newer version (`pacman -Qu`, `apt list --upgradable`, `dnf -C check-update`, `zypper lu`, `brew outdated`, winget), and `Furnish/Installer` when the collection ships a `.deb` or `.rpm` with a different version. The desktop app installs updates automatically on launch and with every scheduled check; turn that off per app on its detail page. By default it postpones them while the computer is busy: when the last 5 minutes averaged over 50% CPU, 90% memory or 1000 KB/s network traffic (`loadLimits` in `settings.json`). It doesn't count its own load: samples taken while furnish works are discarded, and its own processes are subtracted. Turn this off on the Collections page or with `furnish settings --load-aware-updates false`. The CLI: `furnish upgrade` (`--all` includes apps with automatic updates off) and `furnish settings --no-auto-update <app>`.

### Kits

A kit is a set of apps from the same collection that users install together, for example everything a new web developer needs. Each kit is a file in the collection's `kits/` directory:

```yaml
# kits/web-developer.yaml
description: Everything a new web developer needs
categories: [Development]
items:
  - app: vscode
    required: true      # always installed with the kit
  - app: nodejs         # optional, selected by default
  - app: docker
    selected: false     # optional, not selected by default
```

When installing a kit, users choose the optional items; required ones can't be deselected. When uninstalling, furnish preselects the kit's installed apps that no other installed kit contains. In the CLI: `furnish install web-developer --without nodejs --with docker`, `furnish uninstall web-developer [--only docker]`.

### Installer files and Git LFS

Collections can ship installers. Store them with [Git LFS](https://git-lfs.com) (`git lfs track "*.msi"`); furnish downloads LFS files when it adds or updates a collection, so users need `git-lfs` installed. Install a file with the bundled `Furnish/Installer` resource, giving the path relative to the collection root:

| File | Platform | Extra properties |
|---|---|---|
| `.deb`, `.rpm` | Linux | none: the package name comes from the file |
| `.pkg` | macOS | `packageId` (see `pkgutil --pkgs`) |
| `.dmg` | macOS | `app`: the `.app` to copy to /Applications |
| `.msi` | Windows | none: the product code comes from the file |
| `.exe` | Windows | `installArgs` (silent switches), `displayName` (as in Apps & features) |

```yaml
resources:
  - name: VPN client
    type: Furnish/Installer
    condition: "[equals(context().os.family, 'Windows')]"
    properties:
      file: installers/vpn-client.msi
```

See [self-service-example](https://github.com/furnish-self-service/self-service-example) for a complete collection.

## Writing apps and resources

- [DSC v3 guide](docs/dsc-v3-guide.md): how DSC v3.3 behaves, verified against its source code, with the consequences for furnish.
- [authoring-dsc-v3 skill](.claude/skills/authoring-dsc-v3/SKILL.md): rules and a tested example for writing DSC v3 documents and resources. Claude Code loads it automatically in this repository. To use it in your own repository, copy `.claude/skills/authoring-dsc-v3/` there.

## Development

Use the cargo aliases `cargo b`, `cargo t` and `cargo r`. A plain `cargo build` fails without PowerShell, because one DSC build script calls `pwsh` in debug builds. The `fast` profile in `Cargo.toml` avoids that. `tests/e2e.sh` runs install and uninstall against a fake package manager.

`vendor/tree-sitter-dscexpression` holds grammar sources that DSC generates at build time. Regenerate them with `scripts/vendor-dsc-grammar.sh <tag>` when you update the pinned DSC tag.

## License

MIT
