# Shared by nixos-apps.nix and nixos-install.nix: the system alice's configuration builds with
# the given apps (apps.json's `apps`), and her flake.nix.
{ pkgs, nixpkgs, nixosModule, furnish }:
{
  system = apps: (nixpkgs.lib.nixosSystem {
    modules = [ (nixosModule { furnish = { version = 1; inherit apps; }; }) ./configuration.nix ];
  }).config.system.build.toplevel;
  userFlake = pkgs.replaceVars ./flake.nix { nixpkgs = nixpkgs.outPath; furnish = furnish.outPath; };
}
