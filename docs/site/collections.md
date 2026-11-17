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

furnish ships resources for installing software (`Furnish/Package`, `Furnish/Installer`, and on NixOS `Furnish/NixOS`) and for configuring it (see [Configuration](#configuration)), and apps can use any other DSC resource too (see [Other DSC resources](#other-dsc-resources)). furnish skips a resource whose `condition` is false, and hides an app that has no resource for the system. To uninstall, it runs the same resources in reverse order with `_exist: false`, unless the app has [removal steps](#removal-steps).

A step fails when its resource fails. This includes a command that exits with a code other than the expected one, also where the resource itself reports success, like `RunCommandOnSet`. The install stops at that step. After the last step, furnish tests the app. If a resource is still not in its desired state, the install fails and names that resource.

### Furnish/Package

Installs a package with the system's package manager: winget on Windows; on macOS and Linux, the first of pacman, apt, dnf, zypper, brew and nix that is installed, which is Homebrew on most Macs.

| Property | Meaning |
|---|---|
| `name` | The package's name |
| `packageManager` | Use this package manager, not the first one found |
| `pacmanName`, `aptName`, `dnfName`, `zypperName`, `brewName`, `nixName` | The package's name with that package manager, if it differs, for example `pacmanName: libreoffice-fresh` |
| `nixpkgs` | The flake to install Nix packages from (default: the registry's `nixpkgs`) |
| `command` | A program the package provides, for example `claude`. If the package manager doesn't have the package but the program is there, the app shows as installed outside furnish |
| `versions` | The versions that are acceptable (see [Required versions](#required-versions)) |

Many tools are also installed some other way: with npm or pip, with the vendor's own installer, or with another package manager. With `command`, such an app shows as installed outside furnish, and furnish neither installs it a second time nor updates or removes it. furnish looks for the program on `PATH`, then in `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin` (on Windows, in `~\.local\bin` and npm's folder in `%APPDATA%`), and never runs it. If the package manager is missing, the app shows as installed when the program is there, and as needing the package manager when it isn't.

### Furnish/Installer

Installs a file from the collection: `.deb`, `.rpm`, `.pkg`, `.dmg`, `.msi` or `.exe`. furnish downloads the file when someone installs an app that uses it, so a computer only stores the installers of its own apps. Git LFS works too and keeps large files out of the repository's history.

```yaml
- name: Company VPN
  type: Furnish/Installer
  properties:
    file: installers/vpn.msi
```

`file` is a path inside the collection. furnish refuses a path that leads outside it, also through a symbolic link, and the resource refuses to install a symbolic link. `versions` lists the versions that are acceptable (see [Required versions](#required-versions)).

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

### Required versions

`Furnish/Package` and `Furnish/Installer` take `versions`, a list of the versions that are acceptable. When a security fix comes out, list the fixed versions, and furnish updates the computers that have an older one:

```yaml
- name: Firefox
  type: Furnish/Package
  properties:
    name: firefox
    versions:
      - ">=128.0.3, <129"
      - "115.15.0esr"
      - regex: '^115\.(1[6-9]|[2-9][0-9])\.'
```

The installed version is acceptable when it matches at least one entry:

| Entry | Matches |
|---|---|
| A fixed version, such as `115.15.0esr` | exactly this version |
| A range, such as `>=128.0.3, <129` | versions for which every comparison holds. A range starts with `>=`, `>`, `<=`, `<`, `=` or `!=`, and commas separate its comparisons |
| `regex: '...'` | versions whose text the regular expression finds a match in. Use `^` and `$` to match the whole text |

Versions compare by the rules of what installs them:

| Installed with | Rules |
|---|---|
| winget | winget's: `1.0` is the same as `1.0.0`, and `1.0-beta` comes before `1.0` |
| Homebrew, `.pkg`, `.dmg`, `.msi`, `.exe` | numbers compare as numbers; `alpha`, `beta`, `pre` and `rc` come before the release, other words such as `esr` after it |
| pacman | `vercmp` |
| apt, `.deb` | `dpkg --compare-versions` |
| dnf, zypper, `.rpm` | rpm's |
| Nix | Nix's `builtins.compareVersions` |

So `1.10` is later than `1.9`, and an epoch such as `1:` makes a version later than every version without one. A release or Debian revision (`-1`, `-0ubuntu1`) only counts when the entry has one too: `>=128.0.3` accepts `128.0.3-1`.

When the installed version isn't acceptable, the app shows "Security update required" and furnish installs the update as soon as it knows, even if the person turned automatic updates off for the app. It doesn't wait for the update interval or for the computer to be idle, only for an install that is already running. furnish installs:

1. the newest version the package manager offers, if it is acceptable;
2. otherwise, with winget, apt, dnf and zypper, which can install a given version, the newest acceptable version they offer;
3. otherwise nothing: the app shows what it needs and what is offered, for example "Needs 128.0.3 or later; winget offers 128.0.1", and [reports](admins.md#reports) record it as a failure.

While an acceptable version is installed, furnish doesn't update it to one that isn't. With `<129`, Firefox 129 waits until the collection accepts it.

For `Furnish/Installer`, the collection's file is what is offered, so it has to be acceptable itself. `furnish check` reports an error when it isn't. It reads the version of `.deb` files (with `dpkg-deb`) and of `.rpm` files, and warns that it can't tell for the others.

furnish never falls back to an earlier version of the app that the current `versions` don't accept (see [When an install fails](using.md#when-an-install-fails)).

Sometimes furnish can't read the installed version: for a package installed outside furnish (see `command`), or one the package manager installed without a version, such as some portable winget packages. furnish then leaves the app as it is and installs nothing over it. The app's page says that furnish can't check its version against `versions`, and where it is installed if it was installed outside furnish. `furnish list` says "version unknown", and [reports](admins.md#reports) record it.

`Furnish/NixOS` takes no `versions`: its packages come from the system's nixpkgs, which decides their version.

### Configuration

Six resources set up configuration: folders, files, parts of files and settings that a tool applies itself. They work the same on Windows, macOS and Linux, so one step covers every system. Each of them can remove what it set up, so an app made of them needs no removal steps.

Paths are absolute or start with `~/`, which furnish replaces with the home folder on every system. Missing parent folders are created. A path that is a symbolic link is written through to its target. If the target can't be written, as with a file that Home Manager links from the Nix store, the step fails and names the file.

The resources that write files have these properties in common:

| Property | Meaning |
|---|---|
| `_exist` | `false` removes the file, or the resource's part of it |
| `backup` | `true` keeps a copy of the file before each change, named `<file>.<date and time in UTC>~` |
| `validate` | A command that checks the new file before it replaces the old one, with `'%s'` for its path, for example `[ssh, -G, -F, '%s', git.acme.example]`. If it exits with another code than 0, the step fails and the file stays as it was |
| `mode` | The file's mode on macOS and Linux, for example `'640'` |
| `private` | `true`: only the user may open the file. On Windows, its access list grants only the user access and inherits nothing; on macOS and Linux, its mode is 600 |

`mode` and `private` are part of the desired state: a file that someone made readable for others shows as an available update, and installing again restores them. A step without them leaves the mode as it is, so for a file that several `Furnish/Value`, `Furnish/Line` or `Furnish/Block` steps edit, `private: true` on one of them is enough. Steps that give the same file different modes would undo each other, so furnish refuses such an app when it loads it. Uninstalling never changes the mode, and these three resources never delete the file: it may hold the user's own settings.

A file that the account checking the app's status may not read, such as a root-only file that the furnish service wrote, counts as set up if it exists: furnish can't compare its contents as that account. The steps preview says so.

A file is replaced in one step, so programs never read half of it. It keeps its mode, owner and encoding, including a byte order mark and Windows line ends.

To tell whether an app is installed, each resource computes the change it would make to the file and reports the desired state when there is none. Installing makes that change. A setting someone changed by hand shows as an available update, and installing again sets it back.

A value that starts with `[` is a DSC expression. Write `[[` for a pattern or text that starts with `[`.

#### Furnish/Directory

A folder. `_exist: false` removes it only when it is empty. `mode` and `private` work as for files, with mode 700 for a private folder.

| Property | Meaning |
|---|---|
| `path` | The folder |

```yaml
- name: VPN folder
  type: Furnish/Directory
  properties:
    path: ~/.config/acme-vpn
    private: true
```

#### Furnish/File

A whole file, with the text in `content` or a copy of `source`, a file in the collection. furnish checks `source` like the `file` of `Furnish/Installer`: it must be in the collection, and furnish downloads it when the app is installed. Without either, the file only has to exist.

```yaml
- name: VPN profile
  type: Furnish/File
  properties:
    path: ~/.config/acme-vpn/acme.ovpn
    source: files/acme.ovpn
    private: true
```

#### Furnish/Block

Lines between two marker comments that name the app, for example `# BEGIN furnish acme/ssh` and `# END furnish acme/ssh`. If the file has the markers, the step replaces what is between them. Otherwise it adds the block at `insert`: `top`, `bottom` (the default), `before: <pattern>` or `after: <pattern>`. The position counts only for that first time, so the block stays where it is, also when someone adds lines above it. Two apps get two blocks. `_exist: false` removes the block with its markers.

| Property | Meaning |
|---|---|
| `path` | The file |
| `content` | The lines between the markers |
| `comment` | What starts a comment in the file, default `#` |
| `insert` | Where the block goes the first time |

```yaml
- name: Git server host keys
  type: Furnish/Block
  properties:
    path: ~/.ssh/known_hosts
    content: |
      git.acme.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq7…
```

#### Furnish/Line

One line, for files where marker comments aren't allowed. Patterns are regular expressions ([syntax](https://docs.rs/regex/latest/regex/#syntax)) and match anywhere in a line unless they use `^` and `$`.

| Property | Meaning |
|---|---|
| `path` | The file |
| `line` | The line |
| `match` | A pattern for the line to replace |
| `before` | The line must come before the first line that matches this pattern |
| `after` | The line must come after the last line that matches this pattern |

Without `match`, a line equal to `line` must exist. The step adds it only if it's missing, so it never adds a second one, and removal deletes every equal line. With `match`, a line that matches must exist and equal `line`: the step replaces the first match, deletes further matches, and adds `line` if nothing matches. `line` must match `match`, or the next install wouldn't find it and would add it again; furnish refuses such an app when it loads it.

`before` and `after` are part of the desired state. A line that someone moved past one of these lines shows as an available update, and installing again moves it back. A missing line goes right after the `after` match, else right before the `before` match, else at the end. ssh reads an `Include` only before the first `Host` or `Match` line:

```yaml
- name: SSH configuration
  type: Furnish/Line
  properties:
    path: ~/.ssh/config
    line: Include config.d/acme.conf
    before: '^\s*(Host|Match)\s'
```

#### Furnish/Value

One value in a JSON, JSONC (JSON with comments), YAML or TOML file. The rest of the file keeps its keys, their order and its comments. A file that doesn't parse stops the step with an error, and furnish leaves it as it is.

| Property | Meaning |
|---|---|
| `path` | The file |
| `format` | `json`, `jsonc`, `yaml` or `toml`; by default from the file's extension |
| `location` | Where the value is: a JSONPath that names one place, such as `$.hosts['git.acme.example'].git_protocol`, or a JSON Pointer such as `/hosts/git.acme.example/git_protocol` |
| `value` | The value |
| `merge` | `replace` (the default), `deep` or `member` |
| `removeIf` | Instead of `value` and `merge`: a list of values. Installing leaves the value at the location as it is, and uninstalling removes it only if it is one of these |

Missing objects on the way to the location are created. Only the value at the location counts for the app's state. `[?@.name=='acme-prod']` selects the element of a list whose `name` is `acme-prod`: the step replaces that element, or adds it at the end if there is none, and `value` must have that `name`. In an object, as some tools write their entries keyed by an ID, the filter selects the member whose value has that `name`: the step replaces that member under its key, and fails if there is none, since a new member needs a key and a filter doesn't give one. To add a member, name its key in `location`. `merge: deep` sets the keys of `value` and keeps the other keys at the location. `merge: member` treats the location as a list that must contain `value` and keeps its other elements. `_exist: false` removes the value at the location, the keys of a deep merge, or the element, the whole element also with `merge: deep`.

A kubeconfig merge takes one step each for the cluster, the user and the context, and leaves the other entries as they are. kubeconfig holds credentials, and tools such as helm warn about one that others can read, so the first step has `private: true`. `merge: deep` keeps what others add to an entry: kubectl writes defaults into the user when it saves the file, such as `env: null`, and people set a context's `namespace`. With `replace`, either would show as a change, and installing would undo it.

Installing leaves `current-context` alone, so the user's choice stays. If it names the app's context when the app is uninstalled, kubectl fails on every command that doesn't name a context, since that context is gone. A step with `removeIf` removes it in that case and keeps any other value. It has nothing to do when the app is installed, so it never makes the app show as not installed or as having an update, and an uninstall that keeps the value says why in the steps preview. In [removal steps](#removal-steps), such a step needs `_exist: false`.

The whole app:

```yaml
metadata:
  furnish:
    name: Acme Kubernetes
    description: Access to the Acme production cluster with kubectl.
    categories: [Development]
resources:
  - name: Acme cluster
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.clusters[?@.name=='acme-prod']"
      private: true
      merge: deep
      value:
        name: acme-prod
        cluster:
          server: https://k8s.acme.example
  - name: Acme user
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.users[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        user:
          exec: { apiVersion: client.authentication.k8s.io/v1beta1, command: kubelogin, args: [get-token] }
  - name: Acme context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.contexts[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        context: { cluster: acme-prod, user: acme-prod }
  - name: Current context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: $.current-context
      removeIf: [acme-prod]
```

#### Furnish/Command

A setting that a tool must apply itself. `test` runs a command and checks its result: the exit code (`exitCode`, default 0), its output (`output`, ignoring spaces and line breaks at the start and end), or a place in its JSON output (`jsonPath`, and `value` to compare with). `set` runs only when the test fails. `remove`, if given, runs on uninstall whatever the test says, since a failing test can also mean a setting that is there but outdated. So `remove` must succeed when there is nothing to remove. Without `remove`, furnish doesn't offer to uninstall the app. Commands are lists of the program and its arguments, run without a shell. A program that isn't installed makes the app show "Needs" with its name.

```yaml
- name: Vault server
  type: Furnish/Command
  properties:
    test:
      command: [bw, config, server]
      output: https://vault.acme.example
    set: [bw, config, server, https://vault.acme.example]
    remove: [bw, config, server, https://vault.bitwarden.com]
```

`jsonPath` takes the same paths as the `location` of `Furnish/Value`. A filter can select several values, from the elements of a list or the members of an object, and the test passes if one of them equals `value`. openvpn3 lists its profiles keyed by their D-Bus path, for example, and this test finds the profile named Acme:

```yaml
- name: VPN profile imported
  type: Furnish/Command
  properties:
    test:
      command: [openvpn3, configs-list, --json]
      jsonPath: "$[?@.name=='Acme'].name"
      value: Acme
    set: [openvpn3, config-import, --config, /etc/acme/acme.ovpn, --name, Acme, --persistent]
```

A failing test can mean that the setting is missing or that it is there but outdated. `present`, with the same properties as `test`, tells the two apart: if `test` fails and `present` passes, the app shows an update instead of "not installed", and it can be uninstalled. Without `present`, a failing test counts as missing.

A VPN connection that NetworkManager imports from a profile is an example. When the collection brings a new profile, the connection is still there, but it was imported from the old one:

```yaml
- name: VPN profile
  type: Furnish/File
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    path: /etc/openvpn/acme-vpn.ovpn
    source: files/acme.ovpn
- name: VPN connection
  type: Furnish/Command
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    # Imported from this profile: the copy made at import time is the same.
    test: { command: [cmp, -s, /etc/openvpn/acme-vpn.ovpn, /var/lib/acme-vpn/imported.ovpn] }
    # The connection exists, from whichever profile.
    present: { command: [nmcli, connection, show, id, Acme VPN] }
    set: [sh, -c, "nmcli connection delete id 'Acme VPN'; nmcli connection import type openvpn file /etc/openvpn/acme-vpn.ovpn && nmcli connection modify acme-vpn connection.id 'Acme VPN' && mkdir -p /var/lib/acme-vpn && cp /etc/openvpn/acme-vpn.ovpn /var/lib/acme-vpn/imported.ovpn"]
    remove: [sh, -c, "nmcli connection delete id 'Acme VPN'; rm -f /var/lib/acme-vpn/imported.ovpn"]
```

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

For `--dry-run` and the steps preview, furnish's own resources describe what they would do. The package and installer resources show their exact commands. The configuration resources name each file and what happens to it, with the number of lines added and removed. `Furnish/Line` also names the line, and `Furnish/Command` shows its test command with the output it expected and got, and whether `present` found an outdated setting. Other resources show their type and properties.

`Furnish/File`, `Furnish/Block`, `Furnish/Line` and `Furnish/Value` also show the file's changed lines, with 3 lines around each change. For `Furnish/Value`, that is the text as furnish writes it, so you see which comments and formatting stay. When its lines are hidden (see below), it shows the value at its location before and after instead. A file that ends without a line break shows a note after its last line, as `git diff` does, instead of a changed line. In the app, the lines are collapsed under the step; `--dry-run` prints them as a unified diff. When someone changed the file by hand, they show what installing again changes back. A step shows at most 200 lines and cuts lines after 200 characters. A file that isn't text shows "binary file, not shown".

The changed lines of a file that may hold secrets aren't shown, only their number. That is a file with `private: true`, a file with a `mode` that doesn't let group and others read it (such as `600`), and a file that already has such a mode or, on Windows, an access list for the user only. A file the user can't read at all (only an administrator may) shows neither.

No other resource may declare one of furnish's types, such as `Furnish/Package`. furnish refuses to run when DSC finds one, whatever its version.

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

Without removal steps, furnish looks at each resource that applies on the computer. A resource can remove if its manifest has a `delete` operation or its `set` handles `_exist` (`handlesExist: true`). furnish's own resources and `Microsoft.Windows/Registry` can, except a `Furnish/Command` without `remove`. If one of the app's resources can't, furnish doesn't offer to uninstall the app: the app's page shows a note instead of the Uninstall button, and `furnish uninstall` refuses. `furnish check` warns about such apps.

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

`furnish check` reads every file of a collection the way furnish does and installs nothing. It reports what would break for the people installing from it: files that don't load, including translations, installers and icons that aren't in the repository, dependencies and kit items that don't exist, conditions and `recommended` expressions that don't evaluate, and apps that have a resource for no system. It also checks `versions`: an entry that doesn't parse is an error, a range no version can match is a warning, and so is an installer file whose version it can't read; an installer file that `versions` doesn't accept is an error. A resource type that DSC doesn't find on the computer running the check is a warning, since it may be installed where the app runs. An app without removal steps whose resources can't remove what they set up is a warning too. Only types DSC finds on the computer running the check are judged. For each app it shows which resources run on Windows, macOS and Linux. For each translation it warns about apps, kits, categories and options the collection doesn't have, and counts what is translated. Missing translations are not errors. It exits with 1 when there are errors.

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
