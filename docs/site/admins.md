---
title: For admins
order: 5
---
# For admins

## The policy

An admin policy decides which collections people get and what they can change. furnish reads it from each system's place for managed settings, which users can't write:

| System | Where |
|---|---|
| Windows | the registry value `Policy` in `HKLM\SOFTWARE\Policies\furnish`, from Group Policy or Intune |
| macOS | managed preferences for `io.github.furnish-self-service.furnish`, from an MDM profile |
| Linux | `/etc/furnish/policy.json` |

```json
{
  "settings": { "updateIntervalMinutes": { "value": 60, "locked": true } },
  "collections": {
    "preset": [{ "name": "acme-it", "url": "git@github.com:acme/it-apps.git" }],
    "usersCanAdd": false,
    "trustedKeys": ["ssh-ed25519 AAAAC3Nz… it-team"]
  },
  "parameters": { "acme-it": { "role": { "value": "developer" } } },
  "collectionSettings": { "acme-it": { "fallback": { "value": "off", "locked": true } } },
  "autoInstall": ["acme-it/vpn"],
  "allowedResourceTypes": ["Microsoft.Windows/Registry"]
}
```

- `settings`: each value is a default people can change, unless it is `locked`. People's own choices are in `settings.json` in furnish's data folder: `%APPDATA%\furnish` on Windows, `~/Library/Application Support/furnish` on macOS, `~/.local/share/furnish` on Linux.
- `collections.preset`: furnish adds these collections for everyone, and nobody can remove them. With `usersCanAdd: false`, people can't add other collections. A collection counts as preset only if its copy comes from exactly the `url` given here, compared as written. People can't add a collection under a preset's name; one they added before, or a copy from an earlier `url`, is replaced by a fresh copy from this `url`.
- `collections.trustedKeys`: preset collections only move to commits signed with one of these SSH keys. Without them, anyone who can push to the repository can change what gets installed. furnish accepts SSH signatures only; a commit signed with an OpenPGP or X.509 key is refused, whatever keys the computer trusts. furnish also verifies the current commit again before each install. A preset collection with trusted keys only moves forward: a new commit must follow the current one in the history, so the server can't bring back an earlier signed commit. To start such a collection over from the server's history on purpose, remove it from `preset`, let furnish run once, and add it again.
- `parameters`: values for options and answers to questions, per collection or per app (`acme-it/gimp`), as defaults or `locked`. A question answered here isn't asked.
- `collectionSettings`: values for one collection of the settings in [Disk use and earlier versions](#disk-use-and-earlier-versions), as defaults or `locked`.
- `autoInstall`: apps furnish installs on its first start, once each. If someone uninstalls one later, it stays uninstalled.
- `osSpecific`: settings for one operating system, grouped by system. Each value is a plain default or `{ "value": …, "locked": true }`, and furnish checks each one on its own. NixOS has `configuration`, `flakeAttribute` and `homeManagerConfiguration`; see [NixOS](#nixos).
- `allowedResourceTypes`: the DSC resource types apps may use besides furnish's own `Furnish/Package` and `Furnish/Installer`. Case doesn't matter. furnish refuses an app with any other type, also inside a group. Without this key, apps may use any resource that DSC finds.

## Starting at sign-in

furnish installs updates only while it runs. With `startAtSignIn` on, which is the default, it starts when people sign in, with the window closed and its icon in the system tray or menu bar. furnish adds the entry for the current user: a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, a LaunchAgent in `~/Library/LaunchAgents` on macOS, `~/.config/autostart/furnish.desktop` on Linux. It updates or removes the entry each time it starts, so a locked policy value applies from the next start on:

```json
{ "settings": { "startAtSignIn": { "value": true, "locked": true } } }
```

On Linux, the tray icon needs libayatana-appindicator, which the furnish packages and the Nix package bring along. GNOME shows tray icons only with the AppIndicator extension. Without a tray, closing the window quits furnish.

## Updates of furnish

furnish looks for new versions of itself when it starts and every `updateIntervalMinutes`. With `updateIntervalMinutes` at `0` and `updateOnLaunch` off, it doesn't look, and Check now on the Settings page looks once. It reads one small file per channel: `https://furnish-self-service.github.io/furnish/alpha/latest.json` for alpha builds. Offline, it shows nothing. `selfUpdate` decides what furnish does with a new version:

| `selfUpdate` | Installed from the Windows or macOS download | Installed with Homebrew, a Linux package or Nix |
|---|---|---|
| `auto` (default) | furnish installs it | furnish shows that it is out and how it arrives |
| `notify` | furnish shows that it is out; people can install it with Update now | the same as `auto` |
| `off` | furnish doesn't look | furnish doesn't look |

To keep the version under your control, for example when Intune or an MDM deploys furnish, lock it. With `notify` locked, people see the new version, and neither the app nor `furnish self-update` installs it:

```json
{ "settings": { "selfUpdate": { "value": "off", "locked": true } } }
```

Only the policy sets `selfUpdate`; a furnish address can't. Settings → About and `furnish settings` show how furnish was installed: from the MSI's entry in the Windows registry, from `furnish.app` and Homebrew's Caskroom on macOS, and on Linux from the package that contains the program or the Nix store.

Each download is signed with furnish's update key (minisign, the format of Tauri's updater). The app contains the public key. It installs nothing whose signature doesn't match, or whose signature names another version than the file says. A copy of an older version can't pass as a newer one.

On Windows, the update is the .msi, which installs for all users. Windows asks for administrator rights unless the person is an administrator. The installer stops the furnish service, if it is installed, and starts it again afterwards. On macOS, furnish replaces `furnish.app` where it is. If the person can't change that folder, macOS asks for an administrator's password. The app isn't signed with a publisher certificate, so after an update macOS may ask again before furnish reads its sign-ins from the keychain.

## Languages

`language` sets the app's language: `"system"` (the default) or one of `en`, `de`, `fr`, `es`, `it`, `pl`, `pt`, `nl`. `fallbackLanguage` is the language for collection texts that have no translation into the app's language. It is a language tag such as `en` (the default), `de` or `pt-BR`, and may be a language the app itself doesn't come in, as long as your collections translate into it. See [Translations](collections.md#translations).

```json
{ "settings": { "fallbackLanguage": { "value": "de", "locked": true } } }
```

## Disk use and earlier versions

Each person's computer keeps a git copy of every collection. Four settings decide how much of it, and what happens when an install fails. Set them in `settings` for all collections and in `collectionSettings` for one; a value set for a collection wins over a locked general one.

| Setting | Default | Meaning |
|---|---|---|
| `blobsOnDemand` | `true` | Download file contents only when needed: installer files (`.msi`, `.deb`, …) when an app that uses them is installed, earlier versions when furnish falls back to one. The copy holds the history's commits and file lists, which are small. |
| `checkoutDepth` | `0` | Commits of history to keep; `0` keeps all. With `1`, nothing older than the current version is available to fall back to. |
| `fallback` | `"ask"` | After a failed install or update: `"ask"` offers the app's previous version, `"automatic"` installs it, `"off"` does neither. |
| `fallbackMaxAgeDays` | `0` | Fall back only to versions at most this many days older than the failing one; `0`: any age. |

furnish offers the previous version, and the furnish service installs it, only after installing the app's current version failed on that computer. The previous version is the one that last installed on that computer, else the app's file before the collection's latest change to it. furnish installs it from the files of that commit and keeps the app there until the collection changes the app again. With `trustedKeys`, that commit must be signed by one of the keys, like the current one. Turn `fallback` off for collections where an older version must not come back, for example after a security fix, or limit it with `fallbackMaxAgeDays`.

## Sign-in without a VPN

If your collections live on an internal git server, give furnish an address on the internet, such as `furnish.myorg.tld`. People add the address, sign in with your organization's account in the browser, and furnish fetches the collections through it. Nobody needs a VPN for furnish, and the git server stays internal.

The address serves a description at `https://furnish.myorg.tld/.well-known/furnish.json`:

```json
{
  "name": "Acme",
  "issuer": "https://login.myorg.tld/realms/myorg",
  "clientId": "furnish",
  "collections": [{ "name": "acme", "url": "https://furnish.myorg.tld/git/acme.git" }],
  "branding": { "name": "Acme Apps", "accent": "#c2410c" },
  "autoInstall": ["acme/vpn"]
}
```

furnish signs people in with OpenID Connect (authorization code with PKCE; the client is public, furnish holds no secret), keeps the sign-in in the system's credential store, and renews it. It sends the token only to the address. The address must use https, and so must the identity provider; its `/.well-known/openid-configuration` must name exactly the `issuer` of `furnish.json`. A sign-in belongs to the issuer and client ID it came from: if `furnish.json` names another one later, people sign in again.

furnish fetches `furnish.json` again each time it updates all collections: when the app starts (if it updates on launch), at the update interval, and with "Update all" or `furnish collection update`. Changes to the description reach people who added the address themselves without them adding it again. If the address doesn't answer, furnish keeps the description it has. If the new description fails the checks above, furnish also keeps the old one and shows why when people update their collections.

When the address lists a new collection, furnish adds it for people who added the address themselves and asks its questions, as when they add a collection; the address's `parameters` can answer them. It doesn't add a collection whose name a preset collection or one of the person's own collections from another URL has, and says so when people update their collections. A collection the person removed isn't added again, also if the address stops listing it and lists it again later; adding the address again brings it back. For an address the policy lists, new collections become presets.

When the address stops listing a collection, furnish keeps it and the apps installed from it, and people can remove it. The address's defaults no longer apply to it, and its updates fail once the address no longer serves it. For an address the policy lists, the collection stops being a preset.

To make the address's collections presets, list it in the policy: `"endpoints": ["furnish.myorg.tld"]`. With `"usersCanAdd": false` in `collections`, people can only add the addresses the policy lists, and furnish ignores others they added before.

What an address sets works like the policy's settings, as defaults that never lock anything; the policy wins where both set something. An address only sets things for its own collections, the ones its `furnish.json` lists:

| Field | What it may set |
|---|---|
| `autoInstall` | apps of its own collections (`acme/vpn`) |
| `parameters` | values for its own collections and their apps |
| `settings` | `startAtSignIn`, `updateOnLaunch`, `updateIntervalMinutes`, `loadAwareUpdates`, `reporting`, `reportHostname`, `language` and `fallbackLanguage` for everything; `checkoutDepth`, `blobsOnDemand`, `fallback` and `fallbackMaxAgeDays` for its own collections. furnish ignores other keys, such as `selfUpdate`. Give a value plainly (`"reporting": "errors"`) or in the policy's form (`{ "value": "errors" }`, where furnish ignores `locked`). A value that isn't valid for its setting is ignored and the address's other settings still apply; `furnish settings` names what it ignored, and so does the app at the top of its Settings page. |
| `branding` | the app's name and colors, and a logo and tray icon from its own collections. Only one address brands the app: the first one the policy lists that has `branding`; without addresses in the policy, the first address the person added. |

An address the person added doesn't count as owning a collection the policy presets, or a collection of the person's that comes from another URL.

A link like `furnish://add?endpoint=furnish.myorg.tld`, for example in an onboarding email, opens furnish and asks before adding anything. The dialog shows the address, where people sign in (the identity provider's host), the apps the address installs right away, whether it turns reports on, and whether it changes the app's name and colors. Links only add furnish addresses, never git URLs or folders. `furnish collection add furnish.myorg.tld` prints the same before signing in.

Behind the address runs a reverse proxy: it checks the token and forwards only git's two read requests for each collection to exactly one repository on your git server, with a read-only deploy token. The [endpoint setup](endpoint.md) contains the configuration for Caddy, nginx, Traefik, Envoy and HAProxy, and the tests that show what they let through. Decide who may sign in by e-mail domain only if your identity provider marks addresses as verified (`email_verified`); with Microsoft Entra ID, use groups.

## Reports

furnish can report to your organization what it installed, updated or removed on each computer and whether that worked, through your furnish address. Reports are OpenTelemetry logs, sent over OTLP/HTTP. Any OTLP receiver takes them: the OpenTelemetry Collector, Grafana Alloy, Vector, Datadog, Grafana Cloud and others. The address's proxy accepts them from signed-in users only and names the user to the receiver. The receiver adds that user to each record. furnish itself never sends a user name, and one a client puts in a record is dropped. The [endpoint setup](endpoint.md) contains the configuration, with the OpenTelemetry Collector as the receiver.

To look at reports without a log system of your own, add Loki and Grafana to the endpoint: the dashboard "furnish reports" shows failed operations per app, fallbacks to earlier versions, the furnish versions in use and devices that stopped reporting. See [Look at the reports](endpoint.md#look-at-the-reports).

The setting `reporting` decides what furnish sends:

| Value | Reports |
|---|---|
| `off` (default) | none |
| `errors` | failed installs, updates and uninstalls, failed collection updates, and installs of an earlier version after a failure |
| `all` | also successful ones, and once a day the list of installed apps |

People see the setting under Reports to your organization on the Settings page, and `furnish reports` shows what is waiting to be sent.

![The Reports to your organization section, set to send failures and fallbacks.](images/reports-setting.webp)

To turn reports on for everyone, set it in the policy, and lock it if people mustn't turn it off:

```json
{ "settings": { "reporting": { "value": "errors", "locked": true }, "reportHostname": { "value": false } } }
```

The address's `furnish.json` names where reports go, on the address itself: `"events": "https://furnish.myorg.tld/v1/logs"`. It is the full OTLP logs URL, like `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`. Each address gets only the reports about its own collections, and the daily list of installed apps only with its apps; with several addresses, each sees its part. Error messages go out without passwords or tokens: furnish replaces credentials in URLs (`https://user:password@host`), `Bearer` and `Basic` credentials and JWTs with `***`. furnish keeps reports in `reports/queue.ndjson` in its data folder, readable only by the person, until the address accepts them, at most 5 MB, so computers that are offline report later. If the address answers 429, 502, 503 or 504, furnish waits as long as `Retry-After` says, or from 1 minute up to 1 hour, and tries again. Reports the address refuses with 400 or 413 are dropped, so they don't hold up the rest.

Each report is one log record. furnish sets the time it happened, the severity (`ERROR` for failures, `WARN` for fallbacks, `INFO` otherwise), a short text such as `install acme/vpn failed` and these attributes:

| Attribute | Meaning |
|---|---|
| `service.name`, `service.version` | `furnish` and its version (resource) |
| `os.type`, `os.version` | `windows`, `darwin` or `linux`, and the system's version (resource) |
| `device.id` | A random ID furnish created for this user on this computer (resource) |
| `host.name` | The computer's name, only with `reportHostname: true` (resource) |
| `user.email` | The signed-in user's e-mail address, added by the receiver from the proxy's header |
| `furnish.event` | `install`, `update`, `uninstall`, `reinstall`, `fallback`, `collectionUpdate` or `inventory` |
| `furnish.outcome` | `succeeded`, `failed`, `cancelled`; `reported` for an inventory |
| `furnish.app`, `furnish.collection` | The app (`collection/app`) and its collection |
| `furnish.collection.commit` | The collection's commit the app was installed from, or a collection update went to |
| `furnish.trigger` | `user`, `update` (automatic updates) or `autoInstall` |
| `error.message`, `furnish.duration_ms` | The error message of a failure, and how long it took |
| `furnish.apps` | For an inventory: each installed app as `app` and `status` (`installed` or `needsUpdate`) |

Loki, in the endpoint's reports add-on, makes labels of `furnish.event` and `furnish.outcome` and files values other than these under `other`, because a client could otherwise create so many streams that Loki stops taking reports.

Reports about the software people install are personal data, and in Germany a works council has a say before they are collected (§ 87 (1) 6 BetrVG). `errors` without host names collects the least that helps fix broken apps. To send reports without user names, remove the line that sets `X-Furnish-User` from the proxy's configuration.

## Branding

```json
{ "branding": { "name": "Acme Apps", "logo": "acme-it/icons/logo.svg", "accent": "#c2410c" } }
```

The app then shows your name, logo and color. furnish derives the lighter and darker shades of the color and keeps text in it readable in light and dark mode. `accentDark` sets the color for dark mode, and `theme.light` and `theme.dark` set any other color of the app.

![furnish as "Acme Apps" with an orange accent.](images/branded.webp)

The logo can be an SVG, PNG, JPEG or WebP file of up to 1 MB, given as a path inside one of your preset collections or as an absolute path. While furnish runs, the logo also replaces its icon on the window, in the taskbar and the Dock, and in the system tray and the macOS menu bar, in its own colors. The Start menu, pinned taskbar buttons and Launchpad keep furnish's icon, and on Linux so do the app menu and taskbars on Wayland. Text in an SVG logo doesn't show in these icons, so convert it to paths.

A tray icon in one color fits in better next to the other icons there:

```json
{ "branding": { "logo": "acme-it/icons/logo.svg", "trayIcon": "acme-it/icons/tray.svg" } }
```

Draw it in black on a transparent background, as a square or close to it. furnish shows it the way each system shows its own tray icons:

| System | The tray icon |
|---|---|
| Windows | black on a light taskbar, white on a dark one |
| macOS | as a template image: the menu bar colors it for light and dark mode |
| Linux | white, or black when the desktop uses light colors (on GNOME always white, since its top bar is dark) |

furnish uses `trayIcon` if it can read it, otherwise the logo, otherwise its own icon. It follows when the policy or the theme changes while it runs.

## People without administrator rights

furnish normally runs as the person using it and asks for an administrator password when a step needs one. Where people aren't administrators, the furnish service installs the apps of the preset collections for them. Turn it on in the policy, then install the service on each computer:

```json
{ "service": { "allowedGroup": "staff" } }
```

```sh
sudo furnish service install     # on Windows: furnish service install, in an administrator prompt
```

| System | The service | Who may install with it, besides root or SYSTEM |
|---|---|---|
| Windows | service `furnish`, running as SYSTEM | administrators in an elevated session, and members of `allowedGroup` |
| macOS | launchd daemon `io.github.furnish-self-service.furnish` | members of `allowedGroup`, including nested and directory groups |
| Linux | systemd unit `furnish` | members of `allowedGroup`; others need what polkit allows for `io.github.furnish.install` (by default an administrator password) |

`service install` registers the furnish binary it runs from, and refuses one that people without administrator rights could change: the file and the folders above it must belong to root (on Windows: SYSTEM, Administrators or TrustedInstaller) and must not be writable by others. A copy in `~/.cargo/bin`, an AppImage in a home folder, the Homebrew cask or another per-user install doesn't qualify; the furnish packages install to folders that do.

The service runs git from the system: `/usr/bin/git` on Linux and macOS (on NixOS, git from the service's `PATH`). On macOS that means Apple's Command Line Tools, which nobody is asked to install on the service's behalf: install them along with furnish, for example with `xcode-select --install` or from your software distribution. The launchd daemon's `PATH` has only system folders, not `/usr/local/bin` or `/opt/homebrew/bin`, since those belong to whoever installed Homebrew. For Homebrew itself, see [macOS: the service and Homebrew](#macos-the-service-and-homebrew).

Everyone can ask the service to set up and update the preset collections. When that fails, only people who may install see why; others get a short message, and the details go to the service's log: the journal of the `furnish` unit on Linux, `/var/log/furnish.log` on macOS, `C:\ProgramData\furnish\service.log` on Windows. The macOS and Windows logs are readable by administrators only.

On Windows, the service manager restarts the service when it crashes or stops with an error. The service refuses to start when `C:\ProgramData\furnish` exists but wasn't created by it: when it is a link, or when anyone besides SYSTEM and Administrators can change it. Remove the folder and start the service again.

The service finds DSC resources only in folders that people without administrator rights can't change. A folder counts if it and every folder above it belong to root (on Windows: SYSTEM, Administrators or TrustedInstaller) and nobody else can write to it, the same rule as for the furnish binary. The service looks in the folder of the DSC resources furnish includes, which is inside furnish's installation, in the folders on its `PATH` and where DSC is installed, and drops the others. A resource in `/opt/homebrew/bin` or in a folder of a user's own `PATH` would otherwise run with full rights. The service also runs programs only from these folders, except `brew`, which it runs as Homebrew's owner.

The service accepts an app's name and option values, nothing else, and checks the values against the app's declared options and the policy. Apps that install into the user's account (`perUser` in their metadata) still install as the user.

### macOS: the service and Homebrew

Homebrew refuses to run as root. So for `Furnish/Package`, the service runs `brew` as the account that owns Homebrew (`/opt/homebrew`, or `/usr/local/Homebrew` on Intel Macs). It never runs `brew` as root and needs no password for this.

Whoever can run programs as that account could take over the service's installs. So the service uses Homebrew only if:

- the account that owns it is an administrator (a member of `admin`),
- nobody else can change it: Homebrew's folder and the folders above it belong to that account or root, others can't write to them, and only the groups `admin` and `wheel` may.

Otherwise apps that need Homebrew show "Needs Homebrew that only administrators can change", with the reason. This happens, for example, when a person without administrator rights installed Homebrew and owns it. An administrator then has to give the folder to an administrator account, for example `sudo chown -R admin-account /opt/homebrew`, or remove Homebrew and install it again as an administrator.

Casks that run an installer package (`.pkg`), and the uninstall steps of many casks, call `sudo` themselves. The service has nobody to type a password, so allow it once:

```sh
sudo furnish service install --homebrew-sudo
```

This checks the same conditions and adds the file `/etc/sudoers.d/furnish-homebrew` (owner root, group wheel, mode 0440, checked with `visudo`):

```
admin-account ALL = (root) NOPASSWD: ALL
```

The rule names the account that owns Homebrew. It allows every command, because Homebrew runs its package installer, file removals and the casks' own scripts through `sudo`, and each of them already gives full control of the computer. A shorter list would protect nothing and would break casks. The rule gives nothing to people without administrator rights, since they can't run programs as that account. It does mean that the account can use `sudo` without its password. If the account belongs to a person, a program that person starts can then become root without asking. To avoid that, let a separate administrator account that nobody logs in to own Homebrew, and use `brew` through it (`sudo -u that-account -H /opt/homebrew/bin/brew …`).

`furnish service install` without `--homebrew-sudo` removes the file again, and so does `furnish service uninstall`. The service also removes it whenever it finds that Homebrew no longer meets the conditions. Without the file, the service still installs formulae and casks that need no `sudo`. Run `service install --homebrew-sudo` again after giving Homebrew to another account.

## NixOS

The flake provides the packages and a NixOS module:

```nix
{
  inputs.furnish.url = "github:furnish-self-service/furnish";
  # in the configuration, with furnish.nixosModules.default imported:
  services.furnish = {
    enable = true;
    policy.collections.preset = [{ name = "acme-it"; url = "https://github.com/acme/it-apps.git"; }];
    service.enable = true;
  };
}
```

On NixOS, furnish installs Nix packages into the user's profile. Through the service it installs them into a shared profile that the module adds to everyone's `PATH`.

Packages from the NixOS configuration (`environment.systemPackages`, `users.users.<name>.packages`) and from Home Manager show as installed, with where they come from. furnish finds them in the symlink farms NixOS and Home Manager build, such as `/run/current-system/sw`, and matches them by the package's name in nixpkgs. It doesn't install such a package a second time, doesn't update it, and refuses to uninstall it: you change it in the configuration. The app's page shows the package to remove there, for example `pkgs.jq`.

### Where the configuration is

Tell furnish where the configuration is through the module:

```nix
services.furnish.nix = {
  configuration = "/home/alex/cfg";  # a string, so it stays the real folder and not a copy in the Nix store
  flakeAttribute = "alex-laptop";    # default: networking.hostName
};
```

The module writes these to `osSpecific.nixos` in `/etc/furnish/policy.json`, together with `services.furnish.policy`. It also writes `/etc/furnish/nixos.json` with what it knows when the system is built: the host name, `system.configurationRevision` if set, whether Home Manager's NixOS module is in use, and its own version. It restarts the furnish service when the policy changes. A flake can't name its own folder, because Nix evaluates a copy in the store, so the folder only comes from this option. Both values win over people's own setting, which the Settings page then shows as read-only. People see the result on the Settings page and with `furnish settings`.

Without the option, people can set the folder in the app. Without either, furnish searches, in this order, and takes the first configuration that passes a quick check:

1. `/etc/nixos/flake.nix`, following a symbolic link, as `nixos-rebuild` does;
2. the variables of nh: `NH_OS_FLAKE`, `NH_FLAKE`, `FLAKE`, also as NixOS sets them for sessions;
3. `nixos-config=` in `NIX_PATH`, else `/etc/nixos/configuration.nix` (channels).

The check reads the file and evaluates nothing. A `flake.nix` must mention `nixosConfigurations` and the flake attribute. A folder someone set only has to contain a `flake.nix` or `configuration.nix`. If the system reports `system.configurationRevision`, furnish compares it with the folder's git commit and says when they differ. When furnish found the folder by searching, the Settings page offers the line to add to the configuration. Folders such as `~/nixos-config` or `~/dotfiles` are only offered as suggestions.

The Home Manager configuration comes from `osSpecific.nixos.homeManagerConfiguration`, `HOME_MANAGER_CONFIG` or `~/.config/home-manager`.

The furnish service ignores people's own settings for this. It takes the location from the policy or from places only root can change, such as `/etc/nixos`. The folder there can still belong to a person, for example when `/etc/nixos/flake.nix` links into a home folder. furnish passes the folder's owner to its resources (see [Settings available to resources](collections.md#settings-available-to-resources)), so a later rebuild as root can refuse a configuration that isn't root's.

Builds of the `main` branch are in the binary cache `furnish.cachix.org`, which the flake offers. Nix asks once whether to trust the flake's settings. Or add the cache to your configuration:

```nix
nix.settings.extra-substituters = [ "https://furnish.cachix.org" ];
nix.settings.extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
```

Without the cache, and for revisions other than `main`, Nix builds furnish from source. The CLI and the desktop app share one build of their Rust dependencies, so furnish's dependencies are compiled once.

