---
title: Organization sign-in
order: 33
group: Administrators
previous: admins
---
# Organization sign-in

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

## Onboarding links

A link like `furnish://add?endpoint=furnish.myorg.tld`, for example in an onboarding email, opens furnish and asks before adding anything. The dialog shows the address, where people sign in (the identity provider's host), the apps the address installs right away, whether it turns reports on, and whether it changes the app's name and colors. Links only add furnish addresses, never git URLs or folders. `furnish collection add furnish.myorg.tld` prints the same before signing in.

For app and kit actions, see [Share an app or kit](using.md#share-an-app-or-kit).
