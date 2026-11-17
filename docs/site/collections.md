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
locales/<lang>.yaml      # translations, one file per language
icons/                   # images the files above refer to
```

furnish clones the repository for each user and pulls changes at start and every 30 minutes. It downloads the history's commits, but file contents only for the current version; earlier versions and installer files come when they are needed. The [example collection](https://github.com/furnish-self-service/self-service-example) uses everything on this page with real apps.

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
    condition: "[not(equals(context().os.family, 'Windows'))]"
    properties:
      name: gimp
  - name: GIMP (winget)
    type: Furnish/Package
    condition: "[equals(context().os.family, 'Windows')]"
    properties:
      name: GIMP.GIMP.3
```

furnish ships three resources, `Furnish/Package`, `Furnish/Installer` and `Furnish/NixOS`, and apps can use any other DSC resource too (see [Other DSC resources](#other-dsc-resources)). furnish skips a resource whose `condition` is false, and hides an app that has no resource for the system. To uninstall, it runs the same resources in reverse order with `_exist: false`, unless the app has [removal steps](#removal-steps).

A step fails when its resource fails. This includes a command that exits with a code other than the expected one, also where the resource itself reports success, like `RunCommandOnSet`. The install stops at that step. After the last step, furnish tests the app. If a resource is still not in its desired state, the install fails and names that resource.

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

### Furnish/NixOS

Adds packages and NixOS options to the computer's NixOS configuration and rebuilds the system, through the furnish service. The configuration has to be a flake set up for it (see [NixOS](admins.md#installing-apps-into-the-configuration)); otherwise the app shows "Needs ...".

```yaml
- name: htop
  type: Furnish/NixOS
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    name: htop
    packages: [htop]
    options:
      programs.htop.enable: true
```

| Property | Meaning |
|---|---|
| `name` | The app's name in furnish's list on the computer; unique among your apps (letters, digits, `_` and `-`) |
| `packages` | Attribute names in the system's nixpkgs, such as `htop` or `kdePackages.kdenlive`, added to `environment.systemPackages` |
| `options` | NixOS options by name and value. Each needs an entry in the computer's `services.furnish.nix.allowedOptions` |

Each install or uninstall rebuilds the system once per `Furnish/NixOS` resource, so keep an app's packages and options in one.

### Other DSC resources

An app can also use other DSC resources. An app that sets up a VPN connection or a setting needs no package at all.

```yaml
- name: Acme proxy
  type: Microsoft.Windows/Registry
  condition: "[equals(context().os.family, 'Windows')]"
  properties:
    keyPath: HKCU\Software\Acme
    valueName: Proxy
    valueData: { String: proxy.acme.example }
```

furnish includes the resources of DSC 3.3.0, so computers don't need DSC installed for them. Among them:

| System | Resources |
|---|---|
| Windows | `Microsoft.Windows/Registry`, `Microsoft.Windows/EnvironmentVariable`, `Microsoft.Windows/Service`, `Microsoft.Windows/FirewallRuleList`, `Microsoft.Windows/WindowsFeatureList`, `Microsoft.Windows/OptionalFeatureList`, `Microsoft.DSC.Transitional/WindowsPowerShellScript`, `Microsoft.Windows/WindowsPowerShell` (Windows PowerShell DSC resources) |
| All systems | `Microsoft.Filesystem.File/Content`, `Microsoft.DSC.Transitional/RunCommandOnSet`, `Microsoft.DSC/Group`, `Microsoft.DSC/Include`, `Microsoft.DSC/Assertion` |
| Where PowerShell 7 is installed | `Microsoft.DSC.Transitional/PowerShellScript`, `Microsoft.DSC/PowerShell` (PowerShell DSC resources) |

Microsoft's [resource reference](https://learn.microsoft.com/powershell/dsc/reference/resources/builtin?view=dsc-3.0) describes their properties.

furnish also finds resources where DSC does: in the folders on `PATH` and where DSC is installed. A resource the computer doesn't have makes the app show "Needs the DSC resource ...". So install the resource on the computers that need it, for example as a dependency. A resource whose `condition` is false on a computer doesn't need to be there. A folder elsewhere that declares a type furnish includes, such as an installed DSC of another version, isn't searched for resources: furnish's version is the one apps get.

furnish uses `test` to tell whether an app is installed. A resource that is not in the desired state and doesn't report `_exist` counts as not installed. Some resources always report the desired state, such as `RunCommandOnSet`. Combine those with a resource that can tell. The same test runs right after an install, so a resource whose `test` doesn't match what its `set` does makes every install fail.

For `--dry-run` and the steps preview, furnish's own resources show their exact commands. Other resources show their type and properties.

No other resource may declare `Furnish/Package`, `Furnish/Installer` or `Furnish/NixOS`. furnish refuses to run when DSC finds one, whatever its version.

Your organization can limit which resource types apps may use (see [For admins](admins.md#the-policy)).

### Settings available to resources

furnish passes its settings to every resource it runs as environment variables, so your own resources and commands can use them. `FURNISH_CONFIG__` variables hold every setting with the value in effect, as `furnish settings` shows it. `FURNISH_DETECTED__` variables hold what furnish found out, such as where the NixOS configuration is.

The name is the setting's path in `settings.json`. Levels are separated by `__`, and the words of a name by `_`, in capitals:

| Setting | Variable |
|---|---|
| `updateOnLaunch` | `FURNISH_CONFIG__UPDATE_ON_LAUNCH` |
| `osSpecific.nixos.flakeAttribute` | `FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE` |

Texts are passed as they are, `true` and `false` as words, numbers as digits, and lists and maps as JSON. A value nobody set and furnish doesn't know is empty. Option values aren't passed: an app gets its own as parameters (see [Questions and options](#questions-and-options)).

On NixOS, furnish passes where the configuration is, whether a setting or a search found it:

| Variable | Value |
|---|---|
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION` | the configuration's folder |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_KIND` | `flake` or `channels` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_SOURCE` | `module` (furnish's NixOS module), `policy`, `setting`, `etc-nixos`, `nh` or `nix-path` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_OWNER` | the folder's owner, as `<uid>:<user>` |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_OTHERS_CAN_WRITE` | `true` if its group or everyone can write to it |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE` | the name in `nixosConfigurations`, by default the host name |
| `FURNISH_DETECTED__OS_SPECIFIC__NIXOS__HOME_MANAGER_CONFIGURATION` | the Home Manager configuration's folder |

```yaml
- name: Show the configuration
  type: Microsoft.DSC.Transitional/RunCommandOnSet
  properties:
    executable: sh
    arguments: ["-c", "echo \"$FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION\""]
```

In the furnish service, resources run as root, and the settings come from the admin policy and furnish's defaults only, never from a person's own settings.

### Removal steps

Many resources can't remove what they set up. `File/Content` writes a file but doesn't delete it. `RunCommandOnSet` and the PowerShell script resources run their command or script every time. For such an app, list the steps that remove it under `metadata.furnish.uninstall`. They are DSC resources in the same format as `resources`, with conditions and `dependsOn`:

```yaml
metadata:
  furnish:
    uninstall:
      - name: delete the profile
        type: Microsoft.DSC.Transitional/RunCommandOnSet
        condition: "[equals(context().os.family, 'Linux')]"
        properties:
          executable: rm
          arguments: [-f, /etc/acme/profile.conf]
resources:
  - name: profile
    type: Microsoft.Filesystem.File/Content
    condition: "[equals(context().os.family, 'Linux')]"
    properties:
      path: /etc/acme/profile.conf
      content: "server = vpn.acme.example"
```

To uninstall the app, furnish runs the removal steps in their order instead of the resources. The steps preview and `furnish uninstall --dry-run` show them. A removal step whose command exits with a code other than the expected one fails the uninstall. Afterwards furnish tests the app's resources as usual. If they still find the app, the uninstall fails with a message that says so. The organization's list of allowed resource types and the checks on installer files apply to removal steps too.

Without removal steps, furnish looks at each resource that applies on the computer. A resource can remove if its manifest has a `delete` operation or its `set` handles `_exist` (`handlesExist: true`). `Furnish/Package`, `Furnish/Installer` and `Microsoft.Windows/Registry` can. If one of the app's resources can't, furnish doesn't offer to uninstall the app: the app's page shows a note instead of the Uninstall button, and `furnish uninstall` refuses. `furnish check` warns about such apps.

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

The app shows icons on its light or dark background as they are. For a logo that is hard to see in dark mode, such as dark lines without a background of their own, `iconDark` (`metadata.furnish.iconDark` for apps) names the image to show instead in dark mode. Many projects publish a version of their logo for dark backgrounds that fits:

```yaml
metadata:
  furnish:
    icon: icons/helm.svg
    iconDark: icons/helm-white.svg
```

## Kits

A kit installs several apps of the collection together:

```yaml
# kits/office.yaml
name: Office              # shown instead of the ID; optional
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

## Translations

A collection can translate its texts into any language. Each language has one file, `locales/<lang>.yaml`, named with a language tag such as `de`, `fr` or `pt-BR`:

```yaml
# locales/de.yaml
categories:                # a category as the apps write it, and its name
  Office: Büro
apps:
  thunderbird:             # the app's ID
    name: Thunderbird
    description: E-Mail, Kalender und Kontakte.
kits:
  office:
    name: Büro
    description: Office-Paket, E-Mail und Passwortmanager
parameters:                # options and questions of collection.yaml and of the apps
  role:
    label: Woran arbeitest du?
    description: Empfiehlt die Apps für deine Rolle.
    allowedValues: { developer: Softwareentwicklung, design: Design und Medien, office: Büroarbeit }
```

Every entry is optional. The texts in the app files, kit files and `collection.yaml` are in English unless `collection.yaml` names another language, for example `language: de`.

For each text, furnish shows the first of these that exists:

1. the translation into the app's language, the one set under Language or the system's,
2. the translation into the fallback language, English unless people or the policy choose another,
3. the text as written in the collection.

A translation file for `pt` also serves people who use `pt-BR`, and the other way round. If the app's language or the fallback language is the collection's own `language`, furnish shows the collection's texts. A collection written in German with `language: de` shows its texts to German speakers, and to everyone else when it has no translation for them.

The sidebar lists each category once, even if several collections use it. If they translate it differently, the sidebar uses the translation of the first collection in alphabetical order. Search looks at the texts in the app's language, in the fallback language and as written in the collection, so an English word finds an app on a German screen. Reports and logs use IDs, and so do commands such as `furnish install`. `furnish list` shows the IDs with the translated texts.

## Check a collection

`furnish check` reads every file of a collection the way furnish does and installs nothing. It reports what would break for the people installing from it: files that don't load, including translations, installers and icons that aren't in the repository, dependencies and kit items that don't exist, conditions and `recommended` expressions that don't evaluate, and apps that have a resource for no system. A resource type that DSC doesn't find on the computer running the check is a warning, since it may be installed where the app runs. An app without removal steps whose resources can't remove what they set up is a warning too. Only types DSC finds on the computer running the check are judged. For each app it shows which resources run on Windows, macOS and Linux. For each translation it warns about apps, kits, categories and options the collection doesn't have, and counts what is translated. Missing translations are not errors. It exits with 1 when there are errors.

```
$ furnish check
✓ collection.yaml  options
✓ gimp             app  Windows: gimp (winget) · macOS: gimp · Linux: gimp
✗ vpn              app  Windows: setup · macOS: – · Linux: –
    error: setup: installers/vpn.msi is not in the collection
! slack            app  Windows: slack (winget) · macOS: slack (Homebrew) · Linux: –
    warning: no categories (metadata.furnish.categories)
✓ office           kit
✓ locales/de.yaml  translation

Translations (texts without one show in the fallback language or the collection's own):
  de: 3 of 3 apps, 1 of 1 kit, 2 of 3 categories, 1 of 1 option

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
