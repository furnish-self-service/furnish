# A computer's NixOS configuration with furnish: the lines from docs/site/admins.md are
# services.furnish.nix.configuration (configuration.nix) and the two furnish inputs below.
# The test replaces the two paths with store paths: the VMs have no network.
{
  inputs.nixpkgs.url = "path:@nixpkgs@";
  inputs.furnish.url = "path:@furnish@";
  inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";

  outputs = { nixpkgs, furnish, ... }: {
    nixosConfigurations.machine = nixpkgs.lib.nixosSystem {
      modules = [ furnish.nixosModules.default ./configuration.nix ];
    };
  };
}
