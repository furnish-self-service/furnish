# A computer's NixOS configuration with Home Manager as a NixOS module and furnish: the lines
# from docs/site/admins.md are the furnish inputs below, services.furnish.nix
# (../nixos-apps/configuration.nix) and home-manager.users.alice.services.furnish (nixos-home.nix).
# furnish's NixOS module adds its Home Manager module itself. The test replaces the store paths:
# the VM has no network.
{
  inputs.nixpkgs.url = "path:@nixpkgs@";
  inputs.home-manager.url = "path:@homeManager@";
  inputs.home-manager.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.url = "path:@furnish@";
  inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";

  outputs = { nixpkgs, home-manager, furnish, ... }: {
    nixosConfigurations.machine = nixpkgs.lib.nixosSystem {
      modules = [ furnish.nixosModules.default home-manager.nixosModules.home-manager ./configuration.nix ./nixos-home.nix ];
    };
  };
}
