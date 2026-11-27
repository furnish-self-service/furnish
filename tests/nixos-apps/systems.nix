# Shared by nixos-apps.nix, nixos-install.nix and nixos-switch.nix: the system alice's
# configuration builds with the given apps (apps.json's `apps`), also with more of her modules
# (`systemWith`), and her flake.nix.
{ pkgs, nixpkgs, nixosModule, furnish }:
rec {
  systemWith = modules: apps: (nixpkgs.lib.nixosSystem {
    modules = [ (nixosModule { furnish = { version = 1; inherit apps; }; }) ./configuration.nix ] ++ modules;
  }).config.system.build.toplevel;
  system = systemWith [ ];
  userFlake = pkgs.replaceVars ./flake.nix { nixpkgs = nixpkgs.outPath; furnish = furnish.outPath; };
}
