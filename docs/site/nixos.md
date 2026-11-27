---
title: Deploy on NixOS
order: 37
group: Administrators
previous: admins
---
# Deploy on NixOS

## NixOS

The flake provides the packages and a NixOS module:

```nix
{
  inputs.furnish.url = "github:furnish-self-service/furnish";
  # in the configuration, with furnish.nixosModules.default imported:
  services.furnish = {
    enable = true;
    policy.collections.preset = [{ name = "acme-it"; url = "https://github.com/acme/it-apps.git"; }];
    policy.collections.allowedUrlPatterns = [ "https://github[.]com/acme/[A-Za-z0-9_-]+[.]git" ];
    service.enable = true;
  };
  # furnish's binary cache: without it, Nix builds furnish from source on every update.
  nix.settings.extra-substituters = [ "https://furnish.cachix.org" ];
  nix.settings.extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
}
```

Builds of the `main` branch are in the cache. The two `nix.settings` lines are needed because Nix reads a flake's own cache settings (`nixConfig`) only from the flake a command runs on, never from its inputs: `nixos-rebuild` doesn't ask about furnish's cache and doesn't use it. For other revisions, Nix builds furnish from source. The CLI and the desktop app share one build of their Rust dependencies, so they are compiled once.

Settings → About shows the running desktop’s source revision. A Nix package built from a locked Git revision uses a version such as `0.1.0-alpha+git.73b59dac28b3`. Nix exposes the locked commit and its timestamp even when the input URL used a release tag. The GitHub Actions alpha build number is available in Furnish’s published installers. Distributors building a numbered release from its matching source can override both the Nix derivation version and the embedded version with `furnish-gui.overrideAttrs { version = "0.1.0-alpha.332"; FURNISH_VERSION = "0.1.0-alpha.332"; }`. Run `furnish-gui --build-info` to inspect the installed desktop’s version, revision, target and source timestamp without opening a window.

`service.enable` runs the furnish service and turns it on in the policy, so the service installs the apps of the preset collections (see [People without administrator rights](service.md#people-without-administrator-rights)). Members of `policy.service.allowedGroup` install without a password.

When Nix is the selected package manager and the selected configuration has Furnish’s apps input, a new `Furnish/Package` install goes into that configuration. For example, installing `jq` records it in `apps.json` and rebuilds NixOS or activates Home Manager. A collection can use the same package step on other operating systems. The `installInto` setting and the existing service permissions also apply to these installs.

Without a configured apps input, Furnish uses the person's Nix profile, or the shared `/nix/var/nix/profiles/furnish` profile when the service performs the install. Packages already installed in a profile keep that location. Steps that specify their own `nixpkgs` source or require particular versions also use the profile, where Furnish can select the requested build. Packages declared elsewhere in the configuration and programs detected through `command` keep their original ownership.

Packages from the NixOS configuration (`environment.systemPackages`, `users.users.<name>.packages`) and from Home Manager show as installed, with where they come from. furnish finds them in the symlink farms NixOS and Home Manager build, such as `/run/current-system/sw`, and matches them by the package's name in nixpkgs. It doesn't install such a package a second time, doesn't update it, and refuses to uninstall it: you change it in the configuration. The app's page shows the package to remove there, for example `pkgs.jq`.

## Where the configuration is

Tell furnish where the configuration is through the module:

```nix
services.furnish.nix = {
  configuration = "/home/alex/cfg";  # a string, so it stays the real folder and not a copy in the Nix store
  flakeAttribute = "alex-laptop";    # default: networking.hostName
};
```

The module writes these to `osSpecific.nixos` in `/etc/furnish/policy.json`, together with `services.furnish.policy`. It also writes `/etc/furnish/nixos.json` with what it knows when the system is built: the host name, `system.configurationRevision` if set, whether Home Manager's NixOS module is in use, the groups in `nix.trustedGroups` (see [Installing apps into the configuration](#installing-apps-into-the-configuration)), and its own version. When the policy or furnish changes, a switch restarts the furnish service once the service has finished what it is doing, so a switch that the service runs itself doesn't stop it midway. A flake can't name its own folder, because Nix evaluates a copy in the store, so the folder only comes from this option. Both values win over people's own setting, which the Settings page then shows as read-only. People see the result on the Settings page and with `furnish settings`.

Without the option, people can set the folder in the app. Without either, furnish searches, in this order, and takes the first configuration that passes a quick check:

1. `/etc/nixos/flake.nix`, following a symbolic link, as `nixos-rebuild` does;
2. the variables of nh: `NH_OS_FLAKE`, `NH_FLAKE`, `FLAKE`, also as NixOS sets them for sessions;
3. `nixos-config=` in `NIX_PATH`, else `/etc/nixos/configuration.nix` (channels).

The check reads the file and evaluates nothing. A `flake.nix` must mention `nixosConfigurations` and the flake attribute. A folder someone set only has to contain a `flake.nix` or `configuration.nix`. If the system reports `system.configurationRevision`, furnish compares it with the folder's git commit and says when they differ. When furnish found the folder by searching, the Settings page offers the line to add to the configuration. Folders such as `~/nixos-config` or `~/dotfiles` are only offered as suggestions.

The Home Manager configuration comes from furnish's Home Manager module (`services.furnish.nix.configuration` there, see [Installing apps into Home Manager](home-manager.md#installing-apps-into-home-manager)), `osSpecific.nixos.homeManagerConfiguration`, `HOME_MANAGER_CONFIG` or `~/.config/home-manager`.

The furnish service ignores people's own settings for this. It takes the location from the policy or from places only root can change, such as `/etc/nixos`. The folder there can still belong to a person, for example when `/etc/nixos/flake.nix` links into a home folder. furnish passes the folder's owner to its resources (see [Settings available to resources](app-steps.md#settings-available-to-resources)), so a rebuild as root can refuse a configuration that isn't root's (see [Installing apps into the configuration](#installing-apps-into-the-configuration)).

## Installing apps into the configuration

The configuration can hold ordinary Nix packages as well as apps that declare NixOS options. [Writing a collection](nix-resources.md#furnish-nixos) describes the resources. furnish doesn't edit the configuration's files. Add these lines to its `flake.nix` once, next to the furnish input:

```nix
inputs.furnish.url = "github:furnish-self-service/furnish";
inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";
```

`/var/lib/furnish/apps` is a small flake that furnish keeps, with the apps it installed in `apps.json`. The module creates it and adds its packages and options to the system, with no further imports. With `follows`, furnish builds against your nixpkgs, and the apps' packages always come from it. `furnish settings` and the Settings page show the lines when they are missing.

An install needs the furnish service (`services.furnish.service.enable`) and `services.furnish.nix.configuration`. Only the service changes the configuration, and it installs only the apps of preset collections. For an app from a collection that someone added themselves, furnish names the collection and refuses. Preset the collection (`services.furnish.policy.collections.preset`) or list its furnish address in `services.furnish.policy.endpoints`, and set `services.furnish.policy.service.allowedGroup` for people who aren't administrators. The service:

1. writes the app into `apps.json`;
2. runs `nix flake update furnish/apps`, which changes only that entry of `flake.lock`;
3. builds the system and switches to it;
4. commits `flake.lock` if the repository had no other changes. Otherwise it leaves committing to you.

If the build fails, furnish puts `apps.json` and `flake.lock` back. The switch's output is in the journal: `journalctl -u furnish-switch-to-configuration`. Steps 2 to 4 run as the account that owns the configuration's folder; only the switch runs as root. A folder that belongs to a person is rebuilt only when that person or root asks. furnish refuses if anyone else can change the folder or a folder above it, with one exception: a group in `services.furnish.nix.trustedGroups` may.

The check keeps people who can't become root from changing what the service builds and switches to as root. Members of a group that may run any command as root gain nothing from changing the configuration. So by default the list has `wheel` when sudo (`security.sudo` or `security.sudo-rs`) or doas lets `wheel` run any command as root, as the rules NixOS adds for `wheel` do, and is empty otherwise. With `wheelNeedsPassword`, sudo and doas ask for the person's password, and changing the configuration lets them, or a program running as them, have it built as root without that password. A configuration in their own folder allows the same. If the folder belongs to root, steps 2 to 4 run as root, in a repository the group can change.

Its git folder may belong to a member of a trusted group, as when an admin cloned the repository there. Git and Nix refuse to work as root in a repository whose git folder belongs to someone else, so furnish allows this one repository for those steps (git's `safe.directory`) and otherwise uses root's own git settings. What its commit adds to the git folder stays changeable for the group (`core.sharedRepository = group`, unless the repository sets it). furnish refuses if the git folder belongs to someone outside the trusted groups, or if others or a group that isn't trusted can change it. The admins' own git refuses the folder as long as it belongs to root; with `programs.git.enable`, `programs.git.config.safe.directory = [ "/etc/nixos" ];` allows it for everyone on the computer.

Set the list yourself for another group whose members can become root, or to `[ ]` to refuse every folder a group can write to. A folder everyone can write to is always refused, and the setgid bit makes no difference. A configuration that the admins share in `/etc/nixos`:

```nix
systemd.tmpfiles.rules = [ "d /etc/nixos 2775 root wheel -" ];
services.furnish.nix.configuration = "/etc/nixos";
# services.furnish.nix.trustedGroups = [ "wheel" ];  # the default with sudo or doas for wheel
```

By default, an app from a preset collection can set any NixOS option. To restrict the options these apps may set, configure `allowedOptions`.

```nix
services.furnish.nix = {
  allowedOptions = [ "programs.*.enable" "services.flatpak.enable" ];  # * stands for one part of the name
  rebuild = "boot";  # default "switch": switch now; "boot": when the computer starts next
};
```

An explicit list applies to both installation and module evaluation. An empty list allows packages only. Existing lists keep their meaning after an upgrade. Remove the setting to use the unrestricted default.

The module keeps furnish's own source in the system, so the configuration still evaluates offline after a garbage collection. You can also run `nix flake update` yourself: the `furnish/apps` input stays where it points.

`flake.lock` records the folder `/var/lib/furnish/apps`. Evaluating the configuration on another computer, where that folder doesn't exist and the store doesn't have its content, fails with `error: path '/var/lib/furnish/apps' does not exist`. That happens in a CI job that runs `nix flake check`, or on a machine that builds or deploys this configuration. Point the input at furnish's empty app list there:

```sh
nix flake check --override-input furnish/apps 'github:furnish-self-service/furnish?dir=nix/no-apps'
```

The computer itself keeps working, also when the folder is gone, as long as its store has the locked content.
