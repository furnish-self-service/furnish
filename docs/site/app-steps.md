---
title: App conditions and lifecycle
order: 21
group: Collection authors
previous: collections
---
# App conditions and lifecycle

## Changes that need a new login session

When installation changes the environment that a person receives at login, Steps shows a notice before installation. After a successful change, the desktop and CLI ask the person to sign out of the computer and sign back in.

Furnish recognizes changes to Home Manager’s `home.sessionPath` and `home.sessionVariables` and content changes made by `Furnish/File`, `Furnish/Line`, and `Furnish/Block` to standard Unix login profiles such as `~/.profile` and `~/.zprofile`. For a package or custom installer whose changes require a new session, declare this at app level:

```yaml
metadata:
  furnish:
    requiresNewSession: true
```

With this declaration a successful install or update that changes any step produces the notice. An already up-to-date install produces no new notice.

## Host information

Resource steps and dependencies can use the DSC variable `variables('furnish.host')` to choose an install method for the current computer.

| Field | Value |
|---|---|
| `documentsDir` | Documents folder for the account installing the app. Uses the configured XDG Documents directory on Linux (defaults to `~/Documents`), the Documents known folder on Windows (including OneDrive redirection), and `~/Documents` on macOS |
| `distribution` | Linux distribution ID from `os-release`, such as `fedora`, or an empty string when unavailable |
| `distributionLike` | Ordered list of Linux distribution IDs from `ID_LIKE`, or an empty list when unavailable |
| `packageManagers` | List of supported package managers available to the account running Furnish |
| `nixSetup.system` | Boolean. The NixOS configuration is ready to accept apps through Furnish's module |
| `nixSetup.home` | Boolean. The requesting person's Home Manager configuration is ready to accept apps through Furnish's standalone or NixOS-integrated module |

For a personal app, set `metadata.furnish.perUser: true`. To let the person override a location inside their documents folder, give the app a string parameter:

```yaml
parameters:
  location:
    type: string
    defaultValue: "[concat(variables('furnish.host').documentsDir, '/Team docs')]"
```

Both setup fields are `false` on Windows. On macOS only the home destination can be configured. Installing Nix or Home Manager alone leaves the corresponding field `false`. Each field describes readiness independently of the preferred install destination.

For a step specific to Fedora and its derivatives, check both the exact ID and its parents:

```yaml
condition: "[or(equals(variables('furnish.host').distribution, 'fedora'), contains(variables('furnish.host').distributionLike, 'fedora'))]"
```

To prefer Nix wherever it is available, give the Nix step this condition and set `packageManager: nix`. Use the negated condition for the alternative step:

```yaml
condition: "[contains(variables('furnish.host').packageManagers, 'nix')]"
```

## Installing for one person

Set `metadata.furnish.perUser: true` when an app writes files in a person's home folder or calls a tool that stores settings in their account. This includes importing a VPN profile for that person. By default, the furnish service installs apps from an administrator's preset collection as root on Unix or SYSTEM on Windows. In that case, a command can succeed while configuring the service's account.

```yaml
metadata:
  furnish:
    description: Create a private folder for your SSH configuration.
    categories: [Tools]
    perUser: true
resources:
  - name: Personal SSH configuration folder
    type: Furnish/Directory
    properties:
      path: ~/.ssh
      private: true
```

For an administrator's preset collection, a personal app can also include a [`Furnish/NixOS` step](nix-resources.md#furnish-nixos). When the step requires a system rebuild, furnish asks the service to apply that step. With standalone Home Manager, the activation runs as the person. If someone chooses an older app version, its privileged step must still match the declaration in the current preset collection.

If an install option selects between a personal and a system-wide install, declare it as a boolean app parameter and set `perUser: "[parameters('userInstall')]"`. Use the same option in the steps that select the install location.

## Other DSC resources

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

Your organization can limit which resource types apps may use (see [For admins](policies.md#the-policy)).

## Settings available to resources

furnish passes its settings to every resource it runs as environment variables, so your own resources and commands can use them. `FURNISH_CONFIG__` variables hold every setting with the value in effect, as `furnish settings` shows it. `FURNISH_DETECTED__` variables hold what furnish found out, such as where the NixOS configuration is.

The name is the setting's path in `settings.json`. Levels are separated by `__`, and the words of a name by `_`, in capitals:

| Setting | Variable |
|---|---|
| `updateOnLaunch` | `FURNISH_CONFIG__UPDATE_ON_LAUNCH` |
| `osSpecific.nixos.flakeAttribute` | `FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE` |

Texts are passed as they are, `true` and `false` as words, numbers as digits, and lists and maps as JSON. A value nobody set and furnish doesn't know is empty. Option values aren't passed: an app gets its own as parameters (see [Questions and options](catalog.md#questions-and-options)).

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

## Removal steps

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

## Dependencies

An app can depend on other apps of its collection. furnish installs required dependencies first. Optional ones are preselected, and people can deselect them when they install the app.

```yaml
metadata:
  furnish:
    dependencies:
      - app: homebrew
        required: true
        condition: "[equals(context().os.family, 'macOS')]"
      - app: fonts
```

A dependency can have a DSC `condition`, whether it is required or optional. Furnish includes the dependency when the expression is true and skips it when false. In this example, only macOS requires Homebrew, and fonts are optional on every system. Conditions can also use the app’s parameters and the [host information](#host-information) available to resource steps.

If a resource needs a tool that is missing, such as Homebrew on a Mac, the app shows "Needs Homebrew" instead of an Install button. A dependency can install the tool.
