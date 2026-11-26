---
title: Device policy
order: 31
group: Administrators
previous: admins
---
# Device policy

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
- `collectionSettings`: values for one collection of the settings in [Disk use and earlier versions](collection-updates.md#disk-use-and-earlier-versions), as defaults or `locked`.
- `autoInstall`: apps furnish installs on its first start, once each. If someone uninstalls one later, it stays uninstalled.
- `osSpecific`: settings for one operating system, grouped by system. Each value is a plain default or `{ "value": …, "locked": true }`, and furnish checks each one on its own. NixOS has `configuration`, `flakeAttribute`, `homeManagerConfiguration` and `installInto`; see [NixOS](nixos.md#nixos).
- `allowedResourceTypes`: the DSC resource types apps may use besides furnish's own, such as `Furnish/Package`. Case doesn't matter. furnish refuses an app with any other type, also inside a group. Without this key, apps may use any resource that DSC finds.

For an environment with its own package tool, ship a script in a preset collection and register it under `packageManagers` in that collection's `collection.yaml`. The service runs the script from its own checkout when it installs a system app. [Writing a collection](packages.md#package-managers-supplied-by-a-collection)

## Limit the collections people can add

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

Use `selfUpdate` to choose `auto`, `notify` or `off`. To keep deployment under IT control, lock the setting. A locked `notify` shows availability and disables installation through both the desktop and CLI.

```json
{ "settings": { "selfUpdate": { "value": "notify", "locked": true } } }
```

On [self-updating installations](furnish-updates.md), set or lock `updateChannel` independently to `stable` or `alpha`. An unset channel follows the installed build. Only machine policy can lock either setting.

## Languages

`language` sets the app's language: `"system"` (the default) or one of `en`, `de`, `fr`, `es`, `it`, `pl`, `pt`, `nl`. `fallbackLanguage` is the language for collection texts that have no translation into the app's language. It is a language tag such as `en` (the default), `de` or `pt-BR`, and may be a language the app itself doesn't come in, as long as your collections translate into it. See [Translations](catalog.md#translations).

```json
{ "settings": { "fallbackLanguage": { "value": "de", "locked": true } } }
```
