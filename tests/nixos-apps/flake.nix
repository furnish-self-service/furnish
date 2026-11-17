# A computer's NixOS configuration with furnish: the two lines from docs/site/admins.md are
# services.furnish.nix.configuration (configuration.nix) and the furnish/apps input below.
# The test replaces the two paths with store paths: the VM has no network.
{
  inputs.nixpkgs.url = "path:@nixpkgs@";
  inputs.furnish.url = "path:@furnish@";
  inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.inputs.apps.url = "git+file:///var/lib/furnish/apps";

  outputs = { nixpkgs, furnish, ... }: {
    nixosConfigurations.machine = nixpkgs.lib.nixosSystem {
      modules = [ furnish.nixosModules.default ./configuration.nix ];
    };
  };
}
