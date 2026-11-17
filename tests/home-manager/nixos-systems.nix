# The system alice's configuration (nixos-flake.nix) builds with the given apps.json content.
{ pkgs, nixpkgs, nixosModule, furnish, homeManager }:
{
  system = apps: (nixpkgs.lib.nixosSystem {
    modules = [ (nixosModule { furnish = { version = 1; apps = { }; } // apps; }) "${homeManager}/nixos" ../nixos-apps/configuration.nix ./nixos-home.nix ];
  }).config.system.build.toplevel;
  userFlake = pkgs.replaceVars ./nixos-flake.nix {
    nixpkgs = nixpkgs.outPath;
    furnish = furnish.outPath;
    homeManager = homeManager.outPath;
  };
}
