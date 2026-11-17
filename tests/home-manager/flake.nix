# A person's standalone Home Manager configuration with furnish: the lines from
# docs/site/admins.md are the furnish inputs below, the module in `modules` and
# services.furnish (home.nix). alice keeps her data in $XDG_DATA_HOME (~/data). The test replaces
# the store paths: the VM has no network.
{
  inputs.nixpkgs.url = "path:@nixpkgs@";
  inputs.home-manager.url = "path:@homeManager@";
  inputs.home-manager.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.url = "path:@furnish@";
  inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";
  inputs.furnish.inputs.home-apps.url = "path:/home/alice/data/furnish/home-apps";

  outputs = { nixpkgs, home-manager, furnish, ... }: {
    homeConfigurations.alice = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [ furnish.homeManagerModules.default ./home.nix ];
    };
  };
}
