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
- `osSpecific`: settings for one operating system, grouped by system. Each value is a plain default or `{ "value": …, "locked": true }`, and furnish checks each one on its own. NixOS has `configuration`, `flakeAttribute`, `homeManagerConfiguration` and `installInto`; see [NixOS](#nixos).
- `allowedResourceTypes`: the DSC resource types apps may use besides furnish's own, such as `Furnish/Package`. Case doesn't matter. furnish refuses an app with any other type, also inside a group. Without this key, apps may use any resource that DSC finds.

For an environment with its own package tool, ship a script in a preset collection and register it under `packageManagers` in that collection's `collection.yaml`. The service runs the script from its own checkout when it installs a system app. [Writing a collection](collections.md#package-managers-supplied-by-a-collection)

### Limit the collections people can add

Upgrade clients before deploying `collections.allowedUrlPatterns`. Older releases ignore a policy that contains an unknown field.

Set `collections.allowedUrlPatterns` in the machine policy to let people add repositories from approved locations. This example allows HTTPS and SSH repositories in the GitHub organization `acme`. The two patterns cover URLs such as `https://github.com/acme/tools.git` and `git@github.com:acme/tools.git`.

```json
{
  "collections": {
    "usersCanAdd": true,
    "allowedUrlPatterns": [
      "https://github[.]com/acme/[A-Za-z0-9_-]+[.]git",
      "git@github[.]com:acme/[A-Za-z0-9_-]+[.]git"
    ]
  }
}
```

Furnish checks the complete configured Git URL against the list of Rust regular expressions. If Git rewrites the URL, the resulting URL must also match a pattern. To use a mirror, allow both the original address and the mirror address. For a relative local path, Furnish checks the absolute path that Git records. When someone adds a furnish address, every collection URL in its description must match before Furnish accepts that address. Collections preset directly in policy or supplied by an address named in policy remain available.

Omitting `allowedUrlPatterns` allows any collection URL when `usersCanAdd` is true. An empty list or a malformed pattern blocks personal collections. Setting `usersCanAdd` to false continues to block all personal additions. When you tighten the policy, Furnish hides existing personal collections whose URLs no longer match and skips their updates. Their checkouts remain on disk and become available again if the policy allows them.

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

## Release branches and tags

Each collection follows its remote’s default branch until a tracking rule selects another branch or a set of release tags. A named branch follows every new commit on that branch. Tag tracking lets you merge changes throughout the week and publish them to users by tagging the release commit.

To publish stable releases such as `v1.4.0` and `v1.10.2`, put a tracking rule on the preset in the machine policy:

```json
{
  "collections": {
    "preset": [{
      "name": "acme",
      "url": "https://github.com/acme/it-apps.git",
      "tracking": { "mode": "tag", "pattern": "^v[0-9]+[.][0-9]+[.][0-9]+$", "order": "version" }
    }]
  }
}
```

For a staging collection, use `{ "mode": "branch", "name": "staging" }`. Use `{ "mode": "default" }` to follow the remote’s default branch explicitly. Patterns use Rust regular expressions against the short tag name, such as `v1.4.0`. Anchor the pattern with `^` and `$` to match the entire name.

When several tags match, choose how Furnish ranks them.

| `order` | Which tag wins |
| --- | --- |
| `version` (default) | The highest semantic version at the end of the tag name. For example, `v1.10.0` wins over `v1.10.0-rc.2` and `v1.9.9`. Versioned tags take precedence. Other names use natural ordering, so `build-10` follows `build-9`. |
| `newest` | The latest tagger date for an annotated tag or commit date for a lightweight tag. A newly published patch for an older release can win. Equal dates use version ordering. |

Furnish caches tag dates by Git object ID. The first date-based check may download tag and commit metadata for multiple releases. Older Git servers that only serve static files can require downloading the matching tags’ history and file contents. Git records when an annotated tag was created. Use annotated tags if you want to release an existing commit today and have today count as its release date.

Endpoint administrators can put the same `tracking` object in an individual `collections` entry in `furnish.json`. People who add the endpoint themselves can override its default for their personal checkout. A machine-policy preset, including a collection from an endpoint named in policy, uses the organization’s rule for its shared checkout. Its tracking control is locked. For a personal collection, administrators can supply a default or a lock through `collectionSettings.<name>.tracking`, using the existing `value` and `locked` fields.

The rule applies to the first checkout, update notifications and manual or automatic updates. A missing branch or a pattern with no matching tag produces an error and an existing checkout stays usable. Collections that require signed commits still verify the selected commit, including the commit an annotated tag points to. With trusted keys configured, the selected revision must also descend from the installed revision. Publish maintenance lines as separate collections if their histories diverge.

### Review a candidate before publishing a stable tag

On a personal review checkout at the production release, compare a staged branch without changing the configured update source or checkout:

```sh
furnish collection update work --dry-run --branch staging
```

For a particular tag, use an anchored selector such as `--tag-pattern '^v1[.]5[.]0-rc[.]1$'`. Furnish fetches the Git objects it needs, compares both versions’ declarations and identifies steps with changed file inputs. It leaves app installations and the working tree in place. The comparison shows collection definitions for this release pair, so a local app hold, an option or a host condition may give an individual device a different plan.

Machine-managed collections and collections with locked tracking compare against the organization’s selected ref. An administrator can configure a separate staging preset to review its updates under the same signature policy. After publishing the tag, `furnish collection update work --dry-run` shows the release the production checkout would adopt. Pass its two full commit IDs to `furnish collection update work --reviewed CURRENT CANDIDATE` to guard against changes since the review.

## Disk use and earlier versions

Each person's computer keeps a git copy of every collection. Five settings decide how much of it, what happens when an install fails, and whether people can choose an earlier version of an app. Set them in `settings` for all collections and in `collectionSettings` for one; a value set for a collection wins over a locked general one.

| Setting | Default | Meaning |
|---|---|---|
| `blobsOnDemand` | `true` | Download file contents only when needed: installer files (`.msi`, `.deb`, …) when an app that uses them is installed, earlier versions when furnish falls back to one. The copy holds the history's commits and file lists, which are small. |
| `checkoutDepth` | `0` | Commits of history to keep; `0` keeps all. With `1`, nothing older than the current version is available to fall back to. |
| `fallback` | `"ask"` | After a failed install or update: `"ask"` offers the app's previous version, `"automatic"` installs it, `"off"` does neither. |
| `fallbackMaxAgeDays` | `0` | Fall back only to versions at most this many days older than the failing one, and offer only such versions to choose from; `0`: any age. |
| `chooseVersion` | `true` | People can install any earlier version of an app from the history kept, and furnish keeps the app there. |

furnish offers the previous version, and the furnish service installs it, only after installing the app's current version failed on that computer. The previous version is the one that last installed on that computer, else the app's file before the collection's latest change to it. furnish installs it from the files of that commit and keeps the app there until the collection changes the app again. With `trustedKeys`, that commit must be signed by one of the keys, like the current one. An app's [required versions](collections.md#required-versions) apply to earlier versions too: furnish doesn't offer or install an earlier version that has no acceptable version. Turn `fallback` off for collections where an older version must not come back for other reasons, or limit it with `fallbackMaxAgeDays`.

With `chooseVersion`, the app's page and `furnish versions <app>` list the app's versions: the commits within the history kept that changed the app's file, by day, with the versions the file lists in `versions` if it has any. People install one of them, and furnish keeps the app at that version until they choose another or the current one. A later change to the app in the collection doesn't move it; the app's page says that a newer version exists. The same limits as for falling back apply: `checkoutDepth` decides how far back the list goes, `fallbackMaxAgeDays` how much older than the current version a version may be, with `trustedKeys` each commit must be signed by one of the keys, and a version without an acceptable version for the app's [required versions](collections.md#required-versions) isn't listed. When the collection requires a version that the chosen one doesn't offer, furnish installs the required update as for any app: within the chosen version if it offers an acceptable version, else the current version. With `chooseVersion` set to `false`, nobody can choose a version, the furnish service refuses such installs, and apps held at a chosen version follow the collection while it is off. Falling back after a failed install still follows `fallback`. Lock it like the other settings:

```json
{ "collectionSettings": { "acme": { "chooseVersion": { "value": false, "locked": true } } } }
```

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
| `settings` | `startAtSignIn`, `updateOnLaunch`, `updateIntervalMinutes`, `loadAwareUpdates`, `reporting`, `reportHostname`, `language` and `fallbackLanguage` for everything; `checkoutDepth`, `blobsOnDemand`, `fallback`, `fallbackMaxAgeDays` and `chooseVersion` for its own collections. furnish ignores other keys, such as `selfUpdate`. Give a value plainly (`"reporting": "errors"`) or in the policy's form (`{ "value": "errors" }`, where furnish ignores `locked`). A value that isn't valid for its setting is ignored and the address's other settings still apply; `furnish settings` names what it ignored, and so does the app at the top of its Settings page. |
| `branding` | the app's name and colors, and a logo and tray icon from its own collections. Only one address brands the app: the first one the policy lists that has `branding`; without addresses in the policy, the first address the person added. |

An address the person added doesn't count as owning a collection the policy presets, or a collection of the person's that comes from another URL.

A link like `furnish://add?endpoint=furnish.myorg.tld`, for example in an onboarding email, opens furnish and asks before adding anything. The dialog shows the address, where people sign in (the identity provider's host), the apps the address installs right away, whether it turns reports on, and whether it changes the app's name and colors. Links only add furnish addresses, never git URLs or folders. `furnish collection add furnish.myorg.tld` prints the same before signing in.

An app or kit detail page has a **Share an action link** control. Choose Install, Update or Uninstall and copy the link. For example:

- `furnish://install?collection=work&app=editor`
- `furnish://update?collection=work&kit=workstation`
- `furnish://uninstall?collection=work&app=editor`

Each action accepts either `app` or `kit`. Use the collection name as added on the recipient’s computer and the app or kit ID. Percent-encode query values when composing a link manually. Furnish looks up the target locally and asks the person to confirm. A kit install uses its required and preselected apps. An update covers installed apps with an available update. A kit uninstall uses the normal removal defaults, preserving apps shared with other installed kits. The confirmation lists the affected kit members. Installs and updates follow the collection’s dependency rules. If installation requires answers to collection questions, complete them and reopen the link.

If the collection is missing, the action stops. Where policy permits, the person can open Collections to add it separately and then reopen the link to review the action.

Behind the address runs a reverse proxy: it checks the token and forwards only git's two read requests for each collection to exactly one repository on your git server, with a read-only deploy token. The [endpoint setup](endpoint.md) contains the configuration for Caddy, nginx, Traefik, Envoy and HAProxy, and the tests that show what they let through. Decide who may sign in by e-mail domain only if your identity provider marks addresses as verified (`email_verified`); with Microsoft Entra ID, use groups.

## Reports

furnish can report to your organization what it installed, updated or removed on each computer and whether that worked, through your furnish address. Reports are OpenTelemetry logs, sent over OTLP/HTTP. Any OTLP receiver takes them: the OpenTelemetry Collector, Grafana Alloy, Vector, Datadog, Grafana Cloud and others. The address's proxy accepts them from signed-in users only and names the user to the receiver. The receiver adds that user to each record. furnish itself never sends a user name, and one a client puts in a record is dropped. The [endpoint setup](endpoint.md) contains the configuration, with the OpenTelemetry Collector as the receiver.

To look at reports without a log system of your own, add Loki and Grafana to the endpoint: the dashboard "furnish reports" shows failed operations per app, fallbacks to earlier versions, the devices whose installed version of an app isn't acceptable or can't be checked, the furnish versions in use and devices that stopped reporting. See [Look at the reports](endpoint.md#look-at-the-reports).

The setting `reporting` decides what furnish sends:

| Value | Reports |
|---|---|
| `off` (default) | none |
| `errors` | failed installs, updates and uninstalls, failed collection updates, installs of an earlier version after a failure, and installed versions that [required versions](collections.md#required-versions) don't accept |
| `all` | also successful ones, and once a day the list of installed apps |

People see the setting under Reports to your organization on the Settings page, and `furnish reports` shows what is waiting to be sent.

![The Reports to your organization section, set to send failures and fallbacks.](images/reports-setting.webp)

To turn reports on for everyone, set it in the policy, and lock it if people mustn't turn it off:

```json
{ "settings": { "reporting": { "value": "errors", "locked": true }, "reportHostname": { "value": false } } }
```

The address's `furnish.json` names where reports go, on the address itself: `"events": "https://furnish.myorg.tld/v1/logs"`. It is the full OTLP logs URL, like `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`. Each address gets only the reports about its own collections, and the daily list of installed apps only with its apps; with several addresses, each sees its part. Error messages go out without passwords or tokens: furnish replaces credentials in URLs (`https://user:password@host`), `Bearer` and `Basic` credentials and JWTs with `***`. furnish keeps reports in `reports/queue.ndjson` in its data folder, readable only by the person, until the address accepts them, at most 5 MB, so computers that are offline report later. If the address answers 429, 502, 503 or 504, furnish waits as long as `Retry-After` says, or from 1 minute up to 1 hour, and tries again. Reports the address refuses with 400 or 413 are dropped, so they don't hold up the rest.

Each report is one log record. furnish sets the time it happened, the severity (`ERROR` for failures and for `noneOffered`, `WARN` for fallbacks and for `notAcceptable` and `unknown`, `INFO` otherwise), a short text such as `install acme/vpn failed` and these attributes:

| Attribute | Meaning |
|---|---|
| `service.name`, `service.version` | `furnish` and its version (resource) |
| `os.type`, `os.version` | `windows`, `darwin` or `linux`, and the system's version (resource) |
| `device.id` | A random ID furnish created for this user on this computer (resource) |
| `host.name` | The computer's name, only with `reportHostname: true` (resource) |
| `user.email` | The signed-in user's e-mail address, added by the receiver from the proxy's header |
| `furnish.event` | `install`, `update`, `uninstall`, `reinstall`, `fallback`, `collectionUpdate`, `requirement` or `inventory` |
| `furnish.outcome` | `succeeded`, `failed`, `cancelled`; `reported` for an inventory; for a requirement `notAcceptable` (the installed version isn't acceptable, an acceptable one is offered), `noneOffered` (no acceptable version is offered), `unknown` (furnish can't read the installed version, for example of a program installed outside furnish) or `acceptable` (acceptable again) |
| `furnish.app`, `furnish.collection` | The app (`collection/app`) and its collection |
| `furnish.collection.commit` | The collection's commit the app was installed from, or a collection update went to |
| `furnish.trigger` | `user`, `update` (automatic updates) or `autoInstall` |
| `error.message`, `furnish.duration_ms` | The error message of a failure, and how long it took |
| `furnish.apps` | For an inventory: each installed app as `app` and `status` (`installed` or `needsUpdate`) |
| `furnish.installed_version`, `furnish.offered_version` | For `notAcceptable` and `noneOffered`: the installed version and the newest one offered; for `unknown`, the newest one offered |

furnish reports a requirement only when it changes, so a computer that stays on a version that isn't acceptable reports it once. It keeps what it reported last in `reports/requirements.json`. The dashboard's panel of devices on a version that isn't acceptable therefore only lists devices that reported within its time range.

Loki, in the endpoint's reports add-on, makes labels of `furnish.event` and `furnish.outcome` and files values other than these under `other`, because a client could otherwise create so many streams that Loki stops taking reports.

Reports about the software people install are personal data, and in Germany a works council has a say before they are collected (§ 87 (1) 6 BetrVG). `errors` without host names collects the least that helps fix broken apps. To send reports without user names, remove the line that sets `X-Furnish-User` from the proxy's configuration.

## Branding

```json
{ "branding": { "name": "Acme Apps", "logo": "acme-it/icons/logo.svg", "accent": "#c2410c" } }
```

The app then shows your name, logo and color. furnish derives the lighter and darker shades of the color and keeps text in it readable in light and dark mode. `accentDark` sets the color for dark mode, and `theme.light` and `theme.dark` set any other color of the app.

![furnish as "Acme Apps" with an orange accent.](images/branded.webp)

The logo can be an SVG, PNG, JPEG or WebP file of up to 1 MB, given as a path inside one of your preset collections or as an absolute path. While furnish runs, the logo also replaces its icon on the window, in the taskbar and the Dock, and in the system tray and the macOS menu bar, in its own colors. On Linux, after furnish starts, the app menu and Wayland taskbar use the organization's name and logo from a per-user copy of the installed desktop entry. Removing the branding restores the package entry. furnish leaves an existing personal launcher customization in place. The Windows Start menu, pinned Windows taskbar buttons and macOS Launchpad keep furnish's icon. Text in an SVG logo doesn't show in these icons, so convert it to paths.

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

The desktop app and CLI also record failed collection updates in `furnish.log` in the user's data folder. On macOS the default path is `~/Library/Application Support/furnish/furnish.log`, on Linux `~/.local/share/furnish/furnish.log`, and on Windows `%APPDATA%\furnish\furnish.log`. Each entry includes the UTC time, collection name and error. The file stays on the computer even when organization reporting is off. Furnish keeps up to 1 MiB in the current log and one previous file, `furnish.log.1`.

On Windows, the service manager restarts the service when it crashes or stops with an error. The service refuses to start when `C:\ProgramData\furnish` exists but wasn't created by it: when it is a link, or when anyone besides SYSTEM and Administrators can change it. Remove the folder and start the service again.

The service finds DSC resources only in folders that people without administrator rights can't change. A folder counts if it and every folder above it belong to root (on Windows: SYSTEM, Administrators or TrustedInstaller) and nobody else can write to it, the same rule as for the furnish binary. The service looks in the folder of the DSC resources furnish includes, which is inside furnish's installation, in the folders on its `PATH` and where DSC is installed, and drops the others. A resource in `/opt/homebrew/bin` or in a folder of a user's own `PATH` would otherwise run with full rights. The service also runs programs only from these folders, except `brew`, which it runs as Homebrew's owner.

For an install, the service reads the app from its own copy of a preset collection and checks the requested option values against the app's declarations and the policy. [Personal apps](collections.md#installing-for-one-person) run in the user's account. If a personal app needs a system rebuild, the service verifies the requested step against its preset collection before applying a NixOS declaration or adding an ordinary Nix package to the configuration. The app's file and command steps still run as the person installing it.

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
    policy.collections.allowedUrlPatterns = [ "https://github[.]com/acme/[A-Za-z0-9_-]+[.]git" ];
    service.enable = true;
  };
  # furnish's binary cache: without it, Nix builds furnish from source on every update.
  nix.settings.extra-substituters = [ "https://furnish.cachix.org" ];
  nix.settings.extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
}
```

Builds of the `main` branch are in the cache. The two `nix.settings` lines are needed because Nix reads a flake's own cache settings (`nixConfig`) only from the flake a command runs on, never from its inputs: `nixos-rebuild` doesn't ask about furnish's cache and doesn't use it. For other revisions, Nix builds furnish from source. The CLI and the desktop app share one build of their Rust dependencies, so they are compiled once.

Settings → About shows the running desktop’s source revision. A Nix package built from a locked Git revision uses a version such as `0.1.0-alpha+git.73b59dac28b3`. Nix exposes the locked commit and its timestamp even when the input URL used a release tag. The GitHub Actions alpha build number is available in Furnish’s published installers. Distributors building a numbered release from its matching source can override both the Nix derivation version and the embedded version with `furnish-gui.overrideAttrs { version = "0.1.0-alpha.332"; FURNISH_VERSION = "0.1.0-alpha.332"; }`. Run `furnish-gui --build-info` to inspect the installed desktop’s version, revision, target and source timestamp without opening a window.

`service.enable` runs the furnish service and turns it on in the policy, so the service installs the apps of the preset collections (see [People without administrator rights](#people-without-administrator-rights)). Members of `policy.service.allowedGroup` install without a password.

When Nix is the selected package manager and the selected configuration has Furnish’s apps input, a new `Furnish/Package` install goes into that configuration. For example, installing `jq` records it in `apps.json` and rebuilds NixOS or activates Home Manager. A collection can use the same package step on other operating systems. The `installInto` setting and the existing service permissions also apply to these installs.

Without a configured apps input, Furnish uses the person's Nix profile, or the shared `/nix/var/nix/profiles/furnish` profile when the service performs the install. Packages already installed in a profile keep that location. Steps that specify their own `nixpkgs` source or require particular versions also use the profile, where Furnish can select the requested build. Packages declared elsewhere in the configuration and programs detected through `command` keep their original ownership.

Packages from the NixOS configuration (`environment.systemPackages`, `users.users.<name>.packages`) and from Home Manager show as installed, with where they come from. furnish finds them in the symlink farms NixOS and Home Manager build, such as `/run/current-system/sw`, and matches them by the package's name in nixpkgs. It doesn't install such a package a second time, doesn't update it, and refuses to uninstall it: you change it in the configuration. The app's page shows the package to remove there, for example `pkgs.jq`.

### Where the configuration is

Tell furnish where the configuration is through the module:

```nix
services.furnish.nix = {
  configuration = "/home/alex/cfg";  # a string, so it stays the real folder and not a copy in the Nix store
  flakeAttribute = "alex-laptop";    # default: networking.hostName
};
```

The module writes these to `osSpecific.nixos` in `/etc/furnish/policy.json`, together with `services.furnish.policy`. It also writes `/etc/furnish/nixos.json` with what it knows when the system is built: the host name, `system.configurationRevision` if set, whether Home Manager's NixOS module is in use, the groups in `nix.trustedGroups` (see [Installing apps into the configuration](#installing-apps-into-the-configuration)), and its own version. It restarts the furnish service when the policy changes. A flake can't name its own folder, because Nix evaluates a copy in the store, so the folder only comes from this option. Both values win over people's own setting, which the Settings page then shows as read-only. People see the result on the Settings page and with `furnish settings`.

Without the option, people can set the folder in the app. Without either, furnish searches, in this order, and takes the first configuration that passes a quick check:

1. `/etc/nixos/flake.nix`, following a symbolic link, as `nixos-rebuild` does;
2. the variables of nh: `NH_OS_FLAKE`, `NH_FLAKE`, `FLAKE`, also as NixOS sets them for sessions;
3. `nixos-config=` in `NIX_PATH`, else `/etc/nixos/configuration.nix` (channels).

The check reads the file and evaluates nothing. A `flake.nix` must mention `nixosConfigurations` and the flake attribute. A folder someone set only has to contain a `flake.nix` or `configuration.nix`. If the system reports `system.configurationRevision`, furnish compares it with the folder's git commit and says when they differ. When furnish found the folder by searching, the Settings page offers the line to add to the configuration. Folders such as `~/nixos-config` or `~/dotfiles` are only offered as suggestions.

The Home Manager configuration comes from furnish's Home Manager module (`services.furnish.nix.configuration` there, see [Installing apps into Home Manager](#installing-apps-into-home-manager)), `osSpecific.nixos.homeManagerConfiguration`, `HOME_MANAGER_CONFIG` or `~/.config/home-manager`.

The furnish service ignores people's own settings for this. It takes the location from the policy or from places only root can change, such as `/etc/nixos`. The folder there can still belong to a person, for example when `/etc/nixos/flake.nix` links into a home folder. furnish passes the folder's owner to its resources (see [Settings available to resources](collections.md#settings-available-to-resources)), so a rebuild as root can refuse a configuration that isn't root's (see [Installing apps into the configuration](#installing-apps-into-the-configuration)).

### Installing apps into the configuration

The configuration can hold ordinary Nix packages as well as apps that declare NixOS options. [Writing a collection](collections.md#furnishnixos) describes the resources. furnish doesn't edit the configuration's files. Add these lines to its `flake.nix` once, next to the furnish input:

```nix
inputs.furnish.url = "github:furnish-self-service/furnish";
inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";
```

`/var/lib/furnish/apps` is a small flake that furnish keeps, with the apps it installed in `apps.json`. The module creates it and adds its packages and options to the system, with no further imports. With `follows`, furnish builds against your nixpkgs, and the apps' packages always come from it. `furnish settings` and the Settings page show the lines when they are missing.

An install needs the furnish service (`services.furnish.service.enable`) and `services.furnish.nix.configuration`. Only the service changes the configuration, and it installs only the apps of preset collections. For an app from a collection that someone added themselves, furnish names the collection and refuses. Preset the collection (`services.furnish.policy.collections.preset`) or list its furnish address in `services.furnish.policy.endpoints`, and set `services.furnish.policy.service.allowedGroup` for people who aren't administrators. The service:

1. writes the app into `apps.json`;
2. runs `nix flake update furnish/apps`, which changes only that entry of `flake.lock`;
3. builds the system and switches to it;
4. commits `flake.lock` if the repository had no other changes. Otherwise it leaves committing to you.

If the build fails, furnish puts `apps.json` and `flake.lock` back. Steps 2 to 4 run as the account that owns the configuration's folder; only the switch runs as root. A folder that belongs to a person is rebuilt only when that person or root asks. furnish refuses if anyone else can change the folder or a folder above it, with one exception: a group in `services.furnish.nix.trustedGroups` may.

The check keeps people who can't become root from changing what the service builds and switches to as root. Members of a group that may run any command as root gain nothing from changing the configuration. So by default the list has `wheel` when sudo (`security.sudo` or `security.sudo-rs`) or doas lets `wheel` run any command as root, as the rules NixOS adds for `wheel` do, and is empty otherwise. With `wheelNeedsPassword`, sudo and doas ask for the person's password, and changing the configuration lets them, or a program running as them, have it built as root without that password. A configuration in their own folder allows the same. If the folder belongs to root, steps 2 to 4 run as root, in a repository the group can change.

Its git folder may belong to a member of a trusted group, as when an admin cloned the repository there. Git and Nix refuse to work as root in a repository whose git folder belongs to someone else, so furnish allows this one repository for those steps (git's `safe.directory`) and otherwise uses root's own git settings. What its commit adds to the git folder stays changeable for the group (`core.sharedRepository = group`, unless the repository sets it). furnish refuses if the git folder belongs to someone outside the trusted groups, or if others or a group that isn't trusted can change it. The admins' own git refuses the folder as long as it belongs to root; with `programs.git.enable`, `programs.git.config.safe.directory = [ "/etc/nixos" ];` allows it for everyone on the computer.

Set the list yourself for another group whose members can become root, or to `[ ]` to refuse every folder a group can write to. A folder everyone can write to is always refused, and the setgid bit makes no difference. A configuration that the admins share in `/etc/nixos`:

```nix
systemd.tmpfiles.rules = [ "d /etc/nixos 2775 root wheel -" ];
services.furnish.nix.configuration = "/etc/nixos";
# services.furnish.nix.trustedGroups = [ "wheel" ];  # the default with sudo or doas for wheel
```

By default, an app from a preset collection can set any NixOS option. To restrict the options these apps may set, configure `allowedOptions`.

```nix
services.furnish.nix = {
  allowedOptions = [ "programs.*.enable" "services.flatpak.enable" ];  # * stands for one part of the name
  rebuild = "boot";  # default "switch": switch now; "boot": when the computer starts next
};
```

An explicit list applies to both installation and module evaluation. An empty list allows packages only. Existing lists keep their meaning after an upgrade. Remove the setting to use the unrestricted default.

The module keeps furnish's own source in the system, so the configuration still evaluates offline after a garbage collection. You can also run `nix flake update` yourself: the `furnish/apps` input stays where it points.

`flake.lock` records the folder `/var/lib/furnish/apps`. Evaluating the configuration on another computer, where that folder doesn't exist and the store doesn't have its content, fails with `error: path '/var/lib/furnish/apps' does not exist`. That happens in a CI job that runs `nix flake check`, or on a machine that builds or deploys this configuration. Point the input at furnish's empty app list there:

```sh
nix flake check --override-input furnish/apps 'github:furnish-self-service/furnish?dir=nix/no-apps'
```

The computer itself keeps working, also when the folder is gone, as long as its store has the locked content.

### Installing apps into Home Manager

People who keep their programs in Home Manager can get furnish's apps there instead of in the system. furnish's flake has a Home Manager module for this, `homeManagerModules.default`. With the module set up for a person, their Home Manager configuration can receive Nix packages and apps with a `Furnish/NixOS` resource. The setting `osSpecific.nixos.installInto` (`system` or `home`) overrides this, in the policy or on the Settings page. If an app only has options for one of the two (`options` for NixOS, `homeOptions` for Home Manager, see [Writing a collection](collections.md#furnishnixos)), it goes there if that is set up, and otherwise shows "Needs ...". A step with `onlyIfSetUp` is skipped instead.

#### Standalone Home Manager

This works on NixOS and on any other Linux system or Mac with Nix. On the other systems, apps with a `Furnish/NixOS` resource always go into Home Manager, and the Settings page has a Home Manager section instead of the NixOS one.

Add to the Home Manager configuration's `flake.nix`:

```nix
inputs.furnish.url = "github:furnish-self-service/furnish";
inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
inputs.furnish.inputs.home-apps.url = "path:/home/alice/.local/share/furnish/home-apps";

homeConfigurations.alice = home-manager.lib.homeManagerConfiguration {
  inherit pkgs;
  modules = [ furnish.homeManagerModules.default ./home.nix ];
};
```

and to `home.nix`:

```nix
services.furnish.enable = true;
services.furnish.nix.configuration = "/home/alice/.config/home-manager";  # the folder with flake.nix
```

`~/.local/share/furnish/home-apps` (in `$XDG_DATA_HOME` if set, also on macOS) is the person's own small flake with the apps furnish installed for them. furnish creates it when it shows the lines, on the Settings page and with `furnish settings`, so the first `nix flake lock` finds it. A flake input can't use `~`, so the line has the full path. If the input points to another folder, furnish uses that one: it reads the folder from `flake.lock`, or from `flake.nix` before the first lock, so the furnish service finds it too. The module adds the apps' packages to `home.packages`. Without `services.furnish.nix.configuration`, furnish looks in `osSpecific.nixos.homeManagerConfiguration`, `HOME_MANAGER_CONFIG` and `~/.config/home-manager`. `services.furnish.nix.flakeAttribute` is the name in `homeConfigurations`, by default the user name.

An install runs entirely as the person, also when the furnish service runs it for them, and needs no administrator rights. furnish writes the app into the person's `apps.json`, runs `nix flake update furnish/home-apps`, builds the generation and activates it, as `home-manager switch` does. It commits `flake.lock` if the repository had no other changes, and puts `apps.json` and `flake.lock` back if the build fails.

The `furnish/home-apps` input points into the person's home folder, so evaluating the configuration elsewhere fails as described for [the NixOS configuration](#installing-apps-into-the-configuration). Use `--override-input furnish/home-apps 'github:furnish-self-service/furnish?dir=nix/no-apps'` there.

#### Home Manager as a NixOS module

With Home Manager's NixOS module in the configuration, furnish's NixOS module adds its Home Manager module for everyone. Turn it on per person.

```nix
home-manager.users.alice.services.furnish.enable = true;
```

The person's apps go into the system's `/var/lib/furnish/apps`, under their user name, and the service rebuilds the system as described above, with the same rules: it needs the service and the `furnish/apps` input line, and a configuration that belongs to a person is rebuilt only when that person or root asks. Each person gets only their own apps.

Both Home Manager setups allow apps to set Home Manager options by default. To restrict them, set `services.furnish.nix.allowedOptions` in the Home Manager configuration. With Home Manager as a NixOS module, the service reads each person's restriction from the system configuration. Packages come from the configuration's nixpkgs.

The module keeps furnish's own source in the generation. Standalone Home Manager doesn't keep nixpkgs or Home Manager itself, so after a garbage collection, building offline fails until Nix can download them again. To keep them too, pass the inputs to the configuration (`extraSpecialArgs = { inherit nixpkgs home-manager; };`) and add:

```nix
home.extraDependencies = [ nixpkgs home-manager ];
```
