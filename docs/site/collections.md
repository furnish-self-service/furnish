---
title: Writing a collection
order: 4
---
# Writing a collection

A collection is a git repository:

```
collection.yaml          # the collection's icon and questions
apps/<app>.dsc.yaml      # one DSC configuration document per app
kits/<kit>.yaml          # apps installed together
icons/                   # images the files above point to
```

furnish clones it for each user and pulls changes when it starts and every 30 minutes. The [example collection](https://github.com/furnish-self-service/self-service-example) shows all of it with real apps.

## Apps

An app is a standard DSC v3 configuration document. The file name, without `.dsc.yaml`, is the app's name. furnish reads `metadata.furnish` for the catalog and runs `resources` to install:

```yaml
metadata:
  furnish:
    description: Image editor for photo retouching, image composition and painting.
    categories: [Graphics, Photo]
    icon: icons/gimp.svg
resources:
  - name: GIMP
    type: Furnish/Package
    condition: "[not(equals(context().os.family, 'Windows'))]"
    properties:
      name: gimp
  - name: GIMP (winget)
    type: Furnish/Package
    condition: "[equals(context().os.family, 'Windows')]"
    properties:
      name: GIMP.GIMP.3
```

Uninstalling runs the same resources with `_exist: false`, in reverse order. A resource whose `condition` doesn't hold is skipped; an app with no resource for the system isn't shown.

### Furnish/Package

Installs a package with the system's package manager: pacman, apt, dnf, zypper, brew or nix, the first one found; winget on Windows.

| Property | What it does |
|---|---|
| `name` | The package |
| `packageManager` | Use this one instead of the first one found |
| `pacmanName`, `aptName`, `dnfName`, `zypperName`, `brewName`, `nixName` | The package's name with that package manager, when it differs (for example `pacmanName: libreoffice-fresh`) |
| `nixpkgs` | Flake reference to install Nix packages from (default: the registry's `nixpkgs`) |

### Furnish/Installer

Installs a file shipped in the collection: `.deb`, `.rpm`, `.pkg` or `.dmg`, `.msi` or `.exe`. Store large files with Git LFS; furnish fetches them.

```yaml
- name: Company VPN
  type: Furnish/Installer
  properties:
    file: installers/vpn.msi
```

### Dependencies and missing tools

```yaml
metadata:
  furnish:
    dependencies:
      - app: homebrew      # installed first
        required: true
      - app: fonts         # optional: people can deselect it
```

If a resource needs a tool that isn't there, such as Homebrew, the app shows "Needs Homebrew" instead of failing. A dependency can provide it.

## Icons

`metadata.furnish.icon` for apps, `icon` in a kit and in `collection.yaml` for the collection itself: an SVG, PNG, JPEG or WebP file in the repository. The collection's icon shows next to its name on the Collections page, on app cards and on app pages.

## Kits

```yaml
# kits/design-studio.yaml
description: Image editing, vector graphics and digital painting
categories: [Graphics]
icon: icons/design-studio.svg
items:
  - app: gimp
    required: true       # always installed with the kit
  - app: inkscape
    required: true
  - app: krita           # optional, preselected; `selected: false` leaves it out by default
```

![The Design studio kit.](images/kit.webp)

## Questions and options

`collection.yaml` declares options all of a collection's apps can use. A **required** option is a question: furnish asks it when the collection is added and at each start until it is answered.

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

Options with `allowedValues` show as a dropdown. An app uses an option by declaring it as a DSC parameter, and its resources and dependencies use it in `condition`s and properties. Values come from the admin policy first, then from the user (for the app, then for the collection), then from the declared defaults.

## Recommendations

```yaml
metadata:
  furnish:
    recommended: "[equals(parameters('role'), 'design')]"   # or: true
```

Recommended apps show at the top of Discover. The expression can use the collection's options and the app's own.
