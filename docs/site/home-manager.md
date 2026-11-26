---
title: Deploy with Home Manager
order: 38
group: Administrators
previous: admins
---
# Deploy with Home Manager

## Installing apps into Home Manager

People who keep their programs in Home Manager can get furnish's apps there instead of in the system. furnish's flake has a Home Manager module for this, `homeManagerModules.default`. With the module set up for a person, their Home Manager configuration can receive Nix packages and apps with a `Furnish/NixOS` resource. The setting `osSpecific.nixos.installInto` (`system` or `home`) overrides this, in the policy or on the Settings page. If an app only has options for one of the two (`options` for NixOS, `homeOptions` for Home Manager, see [Writing a collection](collections.md#furnishnixos)), it goes there if that is set up, and otherwise shows "Needs ...". A step with `onlyIfSetUp` is skipped instead.

### Standalone Home Manager

This works on NixOS and on any other Linux system or Mac with Nix. On the other systems, apps with a `Furnish/NixOS` resource always go into Home Manager, and the Settings page has a Home Manager section instead of the NixOS one.

Add to the Home Manager configuration's `flake.nix`:

```nix
inputs.furnish.url = "github:furnish-self-service/furnish";
inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
inputs.furnish.inputs.home-apps.url = "path:/home/alice/.local/share/furnish/home-apps";

homeConfigurations.alice = home-manager.lib.homeManagerConfiguration {
  inherit pkgs;
  modules = [ furnish.homeManagerModules.default ./home.nix ];
};
```

and to `home.nix`:

```nix
services.furnish.enable = true;
services.furnish.nix.configuration = "/home/alice/.config/home-manager";  # the folder with flake.nix
```

`~/.local/share/furnish/home-apps` (in `$XDG_DATA_HOME` if set, also on macOS) is the person's own small flake with the apps furnish installed for them. furnish creates it when it shows the lines, on the Settings page and with `furnish settings`, so the first `nix flake lock` finds it. A flake input can't use `~`, so the line has the full path. If the input points to another folder, furnish uses that one: it reads the folder from `flake.lock`, or from `flake.nix` before the first lock, so the furnish service finds it too. The module adds the apps' packages to `home.packages`. Without `services.furnish.nix.configuration`, furnish looks in `osSpecific.nixos.homeManagerConfiguration`, `HOME_MANAGER_CONFIG` and `~/.config/home-manager`. `services.furnish.nix.flakeAttribute` is the name in `homeConfigurations`, by default the user name.

An install runs entirely as the person, also when the furnish service runs it for them, and needs no administrator rights. furnish writes the app into the person's `apps.json`, runs `nix flake update furnish/home-apps`, builds the generation and activates it, as `home-manager switch` does. It commits `flake.lock` if the repository had no other changes, and puts `apps.json` and `flake.lock` back if the build fails.

The `furnish/home-apps` input points into the person's home folder, so evaluating the configuration elsewhere fails as described for [the NixOS configuration](nixos.md#installing-apps-into-the-configuration). Use `--override-input furnish/home-apps 'github:furnish-self-service/furnish?dir=nix/no-apps'` there.

### Home Manager as a NixOS module

With Home Manager's NixOS module in the configuration, furnish's NixOS module adds its Home Manager module for everyone. Turn it on per person.

```nix
home-manager.users.alice.services.furnish.enable = true;
```

The person's apps go into the system's `/var/lib/furnish/apps`, under their user name, and the service rebuilds the system as described in the [NixOS deployment guide](nixos.md), with the same rules: it needs the service and the `furnish/apps` input line, and a configuration that belongs to a person is rebuilt only when that person or root asks. Each person gets only their own apps.

Both Home Manager setups allow apps to set Home Manager options by default. To restrict them, set `services.furnish.nix.allowedOptions` in the Home Manager configuration. With Home Manager as a NixOS module, the service reads each person's restriction from the system configuration. Packages come from the configuration's nixpkgs.

The module keeps furnish's own source in the generation. Standalone Home Manager doesn't keep nixpkgs or Home Manager itself, so after a garbage collection, building offline fails until Nix can download them again. To keep them too, pass the inputs to the configuration (`extraSpecialArgs = { inherit nixpkgs home-manager; };`) and add:

```nix
home.extraDependencies = [ nixpkgs home-manager ];
```
