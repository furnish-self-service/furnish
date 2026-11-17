# Evaluation test: Home Manager as a NixOS module. The furnish service installs a person's apps
# into the system's local flake, under homes.<user>; furnish's Home Manager module gives each
# person only their own. furnish's NixOS module adds the Home Manager module and tells furnish each
# person's allowlist. An option outside the allowlist stops the build. Run:
#   nix build -L .#checks.x86_64-linux.home-manager-nixos
{ pkgs, nixpkgs, nixosModule, furnish }:
let
  inherit (nixpkgs) lib;
  h = import ./home-manager/homes.nix { inherit pkgs nixpkgs furnish; homeManagerModule = null; };
  apps = options: {
    furnish = {
      version = 1;
      apps = { };
      homes.alice.apps.hello = { packages = [ "hello" ]; inherit options; };
    };
  };
  system = options: nixpkgs.lib.nixosSystem {
    modules = [
      (nixosModule (apps options))
      "${h.homeManager}/nixos"
      ./nixos-apps/configuration.nix
      {
        users.users.alice.isNormalUser = true;
        users.users.bob.isNormalUser = true;
        # furnish's NixOS module adds its Home Manager module itself.
        home-manager.sharedModules = [ { home.stateVersion = "25.11"; services.furnish.enable = true; } ];
        home-manager.users.alice.services.furnish.nix.allowedOptions = [ "programs.htop.enable" ];
        home-manager.users.bob = { };
      }
    ];
  };
  good = (system { "programs.htop.enable" = true; }).config;
  bad = (system { "programs.git.enable" = true; }).config;
  names = user: map lib.getName good.home-manager.users.${user}.home.packages;
  facts = builtins.fromJSON good.home-manager.users.alice.xdg.configFile."furnish/home-manager.json".text;
  system-facts = builtins.fromJSON good.environment.etc."furnish/nixos.json".text;
  check = cond: msg: if cond then true else throw "home-manager-nixos: ${msg}";
in
assert check (lib.elem "hello" (names "alice")) "alice has no hello: ${toString (names "alice")}";
assert check (!lib.elem "hello" (names "bob")) "bob has alice's hello";
assert check good.home-manager.users.alice.programs.htop.enable "htop not enabled for alice";
assert check (!good.home-manager.users.bob.programs.htop.enable) "htop enabled for bob";
assert check (!lib.elem "hello" (map lib.getName good.environment.systemPackages)) "hello in the system's packages";
assert check facts.nixosModule "facts don't tell the NixOS module";
assert check (system-facts.homeManagerUsers.alice == { enable = true; allowedOptions = [ "programs.htop.enable" ]; }) "system facts: ${builtins.toJSON system-facts.homeManagerUsers}";
assert check (!(builtins.tryEval bad.system.build.toplevel.drvPath).success) "an option outside the allowlist evaluates";
pkgs.runCommand "furnish-home-manager-nixos" {
  # Evaluates the whole system, without building it.
  system = builtins.unsafeDiscardStringContext good.system.build.toplevel.drvPath;
} "echo $system > $out"
