---
title: Manage collection revisions
order: 32
group: Administrators
previous: admins
---
# Manage collection revisions

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

## Review a candidate before publishing a stable tag

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

furnish offers the previous version, and the furnish service installs it, only after installing the app's current version failed on that computer. The previous version is the one that last installed on that computer, else the app's file before the collection's latest change to it. furnish installs it from the files of that commit and keeps the app there until the collection changes the app again. With `trustedKeys`, that commit must be signed by one of the keys, like the current one. An app's [required versions](versions.md#required-versions) apply to earlier versions too: furnish doesn't offer or install an earlier version that has no acceptable version. Turn `fallback` off for collections where an older version must not come back for other reasons, or limit it with `fallbackMaxAgeDays`.

With `chooseVersion`, the app's page and `furnish versions <app>` list the app's versions: the commits within the history kept that changed the app's file, by day, with the versions the file lists in `versions` if it has any. People install one of them, and furnish keeps the app at that version until they choose another or the current one. A later change to the app in the collection doesn't move it; the app's page says that a newer version exists. The same limits as for falling back apply: `checkoutDepth` decides how far back the list goes, `fallbackMaxAgeDays` how much older than the current version a version may be, with `trustedKeys` each commit must be signed by one of the keys, and a version without an acceptable version for the app's [required versions](versions.md#required-versions) isn't listed. When the collection requires a version that the chosen one doesn't offer, furnish installs the required update as for any app: within the chosen version if it offers an acceptable version, else the current version. With `chooseVersion` set to `false`, nobody can choose a version, the furnish service refuses such installs, and apps held at a chosen version follow the collection while it is off. Falling back after a failed install still follows `fallback`. Lock it like the other settings:

```json
{ "collectionSettings": { "acme": { "chooseVersion": { "value": false, "locked": true } } } }
```
