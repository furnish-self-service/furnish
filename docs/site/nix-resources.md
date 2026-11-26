---
title: NixOS resources
order: 23
group: Collection authors
previous: collections
---
# NixOS resources

## Furnish/NixOS

Adds packages and options to the computer's NixOS configuration and rebuilds the system, through the furnish service, or to the person's Home Manager configuration and activates it. The configuration has to be a flake set up for it (see [NixOS](nixos.md#installing-apps-into-the-configuration) and [Home Manager](home-manager.md#installing-apps-into-home-manager)); otherwise the app shows "Needs ...". Only the furnish service changes the NixOS configuration, and it installs only the apps of the collections the administrator presets. An app from another collection can go only into a standalone Home Manager configuration.

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
| `packages` | Attribute names in the configuration's nixpkgs, such as `htop` or `kdePackages.kdenlive`, added to `environment.systemPackages` or `home.packages` |
| `options` | NixOS options by name and value |
| `homeOptions` | Home Manager options by name and value, used instead of `options` when the app goes into Home Manager |
| `onlyIfSetUp` | `true`: the step applies only where the configuration it goes into is set up for furnish. Elsewhere furnish skips it, as for a false `condition` |

Whether the app goes into the NixOS configuration or Home Manager is up to the computer: Home Manager when furnish's Home Manager module is set up for the person, unless the setting `osSpecific.nixos.installInto` says otherwise. `packages` work in both. An app that sets options should give `options`, `homeOptions` or both. An app with options for only one of them goes there, if that is set up.

By default, an app can set any option supported by its target configuration. If that configuration sets `services.furnish.nix.allowedOptions`, each option must match an entry in its list.

For an option-only step, furnish checks the selected flake's evaluated options when it has no installation record for that name. Option names follow the same dotted-name rules as installation. Values must match as JSON, including list order and every member of a nested object. An explicit `null` requires a present option with a null value. If every requested value matches, the step counts as installed, satisfies dependencies and stays under the configuration's ownership. Installing it skips the rebuild. To remove it, edit the configuration. A step that also requests packages still uses its installation record. If the flake cannot be evaluated offline, furnish uses its normal installation path.

Each install or uninstall rebuilds the system or the Home Manager generation once per `Furnish/NixOS` resource, so keep an app's packages and options in one.

Without `onlyIfSetUp`, an app whose configuration isn't set up shows "Needs ...", also on Windows and macOS. With it, the step is skipped there, including on computers without NixOS, and the rest of the app works as usual. The preview shows the step as not needed and says why. The guard also keeps an existing app entry available for removal when that configuration no longer accepts new apps. Use the host setup fields to select a complementary fallback.

For example, this app adds a directory to the person's login shell path. On other Unix systems the person's shell must read `~/.profile` for the fallback to take effect. If Home Manager owns that file, configure Furnish's module or choose a writable shell startup file that the person's shell sources.

```yaml
metadata:
  furnish:
    description: Add the user's local executables to the login path
    categories: [Shell]
    perUser: true
resources:
  - name: Set the login path through Home Manager
    type: Furnish/NixOS
    properties:
      name: local-bin
      onlyIfSetUp: true
      homeOptions:
        home.sessionPath: ["$HOME/.local/bin"]
  - name: Set the login path in the shell profile
    type: Furnish/Line
    condition: "[and(not(equals(context().os.family, 'Windows')), not(variables('furnish.host').nixSetup.home))]"
    properties:
      path: ~/.profile
      line: 'export PATH="$HOME/.local/bin:$PATH"'
```

A guarded NixOS step can also help another step where Home Manager manages a file. Where Home Manager writes `~/.ssh/config` (`programs.ssh`), the file is a link into the Nix store, and `Furnish/Line` can't change it. The first step has Home Manager write the `Include`. Home Manager puts all includes into one `Include` line before the first `Host`, so the `Line` step accepts an `Include` that names the file among others. After the generation is activated, the `Line` step finds that line and changes nothing. Everywhere else, only the `Line` step runs. Set [`perUser: true`](app-steps.md#installing-for-one-person) on the app so the SSH file belongs to the person installing it:

```yaml
- name: SSH include through Home Manager
  type: Furnish/NixOS
  properties:
    name: acme-ssh
    onlyIfSetUp: true
    homeOptions:
      programs.ssh.includes: [config.d/acme.conf]
- name: SSH include
  type: Furnish/Line
  properties:
    path: ~/.ssh/config
    line: Include config.d/acme.conf
    before: '^\s*(Host|Match)\s'
    accept: '^\s*Include\s(.*\s)?config\.d/acme\.conf(\s|$)'
```

`programs.ssh.includes` takes effect only if the person's configuration sets `programs.ssh.enable = true`. Otherwise Home Manager doesn't write `~/.ssh/config`, and the `Line` step edits the file itself.

On uninstall, a `Line` with `accept` leaves an exact match in a file linked by Home Manager to its generation in the Nix store. The `Furnish/NixOS` step removes the option. With a switch the activated generation updates the file, and with a rebuild for boot the active file stays as it is until activation. In an ordinary SSH config, the `Line` step removes the exact include and keeps other lines.
