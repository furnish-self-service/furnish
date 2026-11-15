---
title: For admins
order: 5
---
# For admins

## The policy

An admin policy decides which collections people get and what they can change. furnish reads it from each system's place for managed settings, which users can't write:

| System | Where |
|---|---|
| Linux | `/etc/furnish/policy.json` |
| macOS | managed preferences for `io.github.furnish-self-service.furnish`, from an MDM profile |
| Windows | the registry value `Policy` in `HKLM\SOFTWARE\Policies\furnish`, from Group Policy or Intune |

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
  "autoInstall": ["acme-it/vpn"]
}
```

- `settings`: each value is a default people can change, unless it is `locked`. People's own choices are in `settings.json` in furnish's data folder: `~/.local/share/furnish` on Linux, `~/Library/Application Support/furnish` on macOS, `%APPDATA%\furnish` on Windows.
- `collections.preset`: furnish adds these collections for everyone, and nobody can remove them. With `usersCanAdd: false`, people can't add other collections.
- `collections.trustedKeys`: preset collections only move to commits signed with one of these SSH keys. Without them, anyone who can push to the repository can change what gets installed.
- `parameters`: values for options and answers to questions, per collection or per app (`acme-it/gimp`), as defaults or `locked`. A question answered here isn't asked.
- `collectionSettings`: values for one collection of the settings in [Disk use and earlier versions](#disk-use-and-earlier-versions), as defaults or `locked`.
- `autoInstall`: apps furnish installs on its first start, once each. If someone uninstalls one later, it stays uninstalled.

## Disk use and earlier versions

Each person's computer keeps a git copy of every collection. Four settings decide how much of it, and what happens when an install fails. Set them in `settings` for all collections and in `collectionSettings` for one; a value set for a collection wins over a locked general one.

| Setting | Default | Meaning |
|---|---|---|
| `blobsOnDemand` | `true` | Download file contents only when needed: installer files (`.msi`, `.deb`, …) when an app that uses them is installed, earlier versions when furnish falls back to one. The copy holds the history's commits and file lists, which are small. |
| `checkoutDepth` | `0` | Commits of history to keep; `0` keeps all. With `1`, nothing older than the current version is available to fall back to. |
| `fallback` | `"ask"` | After a failed install or update: `"ask"` offers the app's previous version, `"automatic"` installs it, `"off"` does neither. |
| `fallbackMaxAgeDays` | `0` | Fall back only to versions at most this many days older than the failing one; `0`: any age. |

The previous version is the one that last installed on that computer, else the app's file before the collection's latest change to it. furnish installs it from the files of that commit and keeps the app there until the collection changes the app again. With `trustedKeys`, that commit must be signed by one of the keys, like the current one. Turn `fallback` off for collections where an older version must not come back, for example after a security fix, or limit it with `fallbackMaxAgeDays`.

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

furnish signs people in with OpenID Connect (authorization code with PKCE; the client is public, furnish holds no secret), keeps the sign-in in the system's credential store, and renews it. It sends the token only to the address. Its `branding`, `settings`, `parameters` and `autoInstall` work like the policy's, as defaults; the policy wins where both set something. To make the address's collections presets, list it in the policy: `"endpoints": ["furnish.myorg.tld"]`.

Behind the address runs a reverse proxy: it checks the token with oauth2-proxy and forwards only git's two read requests for each collection to exactly one repository on your git server, with a read-only deploy token. The [reference setup](https://github.com/furnish-self-service/furnish/tree/main/packaging/endpoint) contains the configuration for Caddy and oauth2-proxy, and the tests that show what it lets through.

A link like `furnish://add?endpoint=furnish.myorg.tld`, for example in an onboarding email, opens furnish and starts the sign-in after the person confirms.

## Branding

```json
{ "branding": { "name": "Acme Apps", "logo": "acme-it/icons/logo.svg", "accent": "#c2410c" } }
```

The app then shows your name, logo and color. furnish derives the lighter and darker shades of the color and keeps text in it readable in light and dark mode. `accentDark` sets the color for dark mode, and `theme.light` and `theme.dark` set any other color of the app.

![furnish as "Acme Apps" with an orange accent.](images/branded.webp)

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
| Linux | systemd unit `furnish` | members of `allowedGroup`; others need what polkit allows for `io.github.furnish.install` (by default an administrator password) |
| macOS | launchd daemon `io.github.furnish-self-service.furnish` | members of `allowedGroup`, including nested and directory groups |
| Windows | service `furnish`, running as SYSTEM | administrators in an elevated session, and members of `allowedGroup` |

The service accepts an app's name and option values, nothing else, and checks the values against the app's declared options and the policy. Apps that install into the user's account (`perUser` in their metadata) still install as the user.

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
