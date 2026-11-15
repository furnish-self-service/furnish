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
icons/                   # images the files above refer to
```

furnish clones the repository for each user and pulls changes at start and every 30 minutes. It downloads the history's commits, but file contents only for the current version; earlier versions and installer files come when they are needed. The [example collection](https://github.com/furnish-self-service/self-service-example) uses everything on this page with real apps.

## Apps

An app is a [DSC v3](https://learn.microsoft.com/powershell/dsc/overview) configuration document. Its file name without `.dsc.yaml` is the app's name. furnish shows what `metadata.furnish` says in the catalog and runs `resources` to install the app:

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

furnish ships two resources, `Furnish/Package` and `Furnish/Installer`, and apps can use any other DSC resource too (see [Other DSC resources](#other-dsc-resources)). furnish skips a resource whose `condition` is false, and hides an app that has no resource for the system. To uninstall, it runs the same resources in reverse order with `_exist: false`.

### Furnish/Package

Installs a package with the system's package manager: winget on Windows; on macOS and Linux, the first of pacman, apt, dnf, zypper, brew and nix that is installed, which is Homebrew on most Macs.

| Property | Meaning |
|---|---|
| `name` | The package's name |
| `packageManager` | Use this package manager, not the first one found |
| `pacmanName`, `aptName`, `dnfName`, `zypperName`, `brewName`, `nixName` | The package's name with that package manager, if it differs, for example `pacmanName: libreoffice-fresh` |
| `nixpkgs` | The flake to install Nix packages from (default: the registry's `nixpkgs`) |

### Furnish/Installer

Installs a file from the collection: `.deb`, `.rpm`, `.pkg`, `.dmg`, `.msi` or `.exe`. furnish downloads the file when someone installs an app that uses it, so a computer only stores the installers of its own apps. Git LFS works too and keeps large files out of the repository's history.

```yaml
- name: Company VPN
  type: Furnish/Installer
  properties:
    file: installers/vpn.msi
```

`file` is a path inside the collection. furnish refuses a path that leads outside it, also through a symbolic link, and the resource refuses to install a symbolic link.

### Other DSC resources

An app can also use resources that DSC finds on the computer: the ones that come with DSC, such as `Microsoft.Windows/Registry` or `Microsoft.DSC.Transitional/RunCommandOnSet`, and resources installed elsewhere. An app that sets up a VPN connection or a setting needs no package at all.

```yaml
- name: Acme proxy
  type: Microsoft.Windows/Registry
  condition: "[equals(context().os.family, 'Windows')]"
  properties:
    keyPath: HKCU\Software\Acme
    valueName: Proxy
    valueData: { String: proxy.acme.example }
```

furnish looks for resources where DSC does: in the folders on `PATH` and where DSC is installed. A resource the computer doesn't have makes the app show "Needs the DSC resource ...". So install DSC, or the resource, on the computers that need it, for example as a dependency. A resource whose `condition` is false on a computer doesn't need to be there.

furnish uses `test` to tell whether an app is installed. A resource that is not in the desired state and doesn't report `_exist` counts as not installed. Uninstalling sets `_exist: false`, so an app whose resources don't support `_exist` can't be uninstalled with furnish. Some resources always report the desired state, such as `RunCommandOnSet`. Combine those with a resource that can tell.

For `--dry-run` and the steps preview, furnish's own resources show their exact commands. Other resources show their type and properties.

No other resource may declare `Furnish/Package` or `Furnish/Installer`. furnish refuses to run when DSC finds one, whatever its version.

Your organization can limit which resource types apps may use (see [For admins](admins.md#the-policy)).

### Dependencies

An app can depend on other apps of its collection. furnish installs required dependencies first. Optional ones are preselected, and people can deselect them when they install the app.

```yaml
metadata:
  furnish:
    dependencies:
      - app: homebrew
        required: true
      - app: fonts
```

If a resource needs a tool that is missing, such as Homebrew on a Mac, the app shows "Needs Homebrew" instead of an Install button. A dependency can install the tool.

## Icons

Apps set `metadata.furnish.icon`, kits and `collection.yaml` set `icon`: the path of an SVG, PNG, JPEG or WebP file in the repository. The collection's icon appears next to its name on the Collections page, on app cards and on app pages.

## Kits

A kit installs several apps of the collection together:

```yaml
# kits/office.yaml
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

## Check a collection

`furnish check` reads every file of a collection the way furnish does and installs nothing. It reports what would break for the people installing from it: files that don't load, installers and icons that aren't in the repository, dependencies and kit items that don't exist, conditions and `recommended` expressions that don't evaluate, and apps that have a resource for no system. A resource type that DSC doesn't find on the computer running the check is a warning, since it may be installed where the app runs. For each app it shows which resources run on Windows, macOS and Linux. It exits with 1 when there are errors.

```
$ furnish check
✓ collection.yaml  options
✓ gimp             app  Windows: gimp (winget) · macOS: gimp · Linux: gimp
✗ vpn              app  Windows: setup · macOS: – · Linux: –
    error: setup: installers/vpn.msi is not in the collection
! slack            app  Windows: slack (winget) · macOS: slack (Homebrew) · Linux: –
    warning: no categories (metadata.furnish.categories)
✓ office           kit

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

Conditions are evaluated for each system by putting the system's name in place of `context().os.family`. Conditions on other details, such as the system's version, are evaluated for the computer `furnish check` runs on.

## Private repositories

A collection can be a private repository:

- Over HTTPS, furnish asks for a username and a password or access token when the server requires one, keeps them in the system's credential store, and sends them to that repository only. On the command line: `furnish collection add <url> --username alice`, which asks for the password, or `--password-stdin` for scripts.
- Over SSH, furnish uses git as it is set up: your keys, `ssh-agent` and `~/.ssh/config`. Connect to the server once with `ssh` to accept its host key.

git's own credential helpers and credentials in the URL also work.

![The add form with the login fields for a private HTTPS repository.](images/private-collection.webp)
