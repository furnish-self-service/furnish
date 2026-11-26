---
title: Write a collection
order: 20
group: Collection authors
---
# Write a collection

A collection is a git repository with this layout:

```
collection.yaml          # the collection's icon and the questions it asks
apps/<app>.dsc.yaml      # one file per app
kits/<kit>.yaml          # apps that are installed together
locales/<lang>.yaml      # translations, one file per language
icons/                   # images the files above refer to
```

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

Furnish evaluates each step’s condition on the target computer. An app with no applicable steps is hidden. Installation stops on a failed step and reports any resource still outside its desired state after the final check. Uninstall applies the steps in reverse order unless you supply [removal steps](app-steps.md#removal-steps).

## Test and publish

Run `furnish check` from the repository root, then add the repository to Furnish and preview the app’s steps. The [example collection](https://github.com/furnish-self-service/self-service-example) shows complete working definitions.

| What you want to do | Guide |
|---|---|
| Install software or require a specific version | [Packages and installers](packages.md) · [Version requirements](versions.md) |
| Configure a machine | [Files and directories](files.md) · [Commands](commands.md) · [NixOS resources](nix-resources.md) |
| Choose steps by platform, set dependencies or customize uninstall | [App conditions and lifecycle](app-steps.md) |
| Group apps or tailor the catalog to different users | [Kits and catalog presentation](catalog.md) |
| Test changes before publishing | [Validation and CI](validation.md) |

## Private repositories

A collection can be a private repository:

- Over HTTPS, furnish asks for a username and a password or access token when the server requires one, keeps them in the system's credential store, and sends them to that repository only. On the command line: `furnish collection add <url> --username alice`, which asks for the password, or `--password-stdin` for scripts.
- Over SSH, furnish uses git as it is set up: your keys, `ssh-agent` and `~/.ssh/config`. Connect to the server once with `ssh` to accept its host key.

git's own credential helpers and credentials in the URL also work.

![The add form with the login fields for a private HTTPS repository.](images/private-collection.webp)
