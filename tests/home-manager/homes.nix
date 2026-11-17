# Shared by the Home Manager tests: Home Manager's source (not an input of furnish's flake, so
# people's flake.lock doesn't get it; fetched as a flake input is, so the generations built here
# and from alice's flake are the same), the generation alice's configuration builds with the given
# apps (apps.json's `apps`), and her flake.nix.
{ pkgs, nixpkgs, homeManagerModule, furnish }:
rec {
  homeManager = builtins.fetchTree {
    type = "github";
    owner = "nix-community";
    repo = "home-manager";
    rev = "3ee51fbdac8c8bdfe1e7e1fcaba6520a563f394f"; # release-25.11
    narHash = "sha256-QOD/CNm196nCJRheux/URi4/HE66fthdOMqCJoPP1Y0=";
  };
  hmLib = import "${homeManager}/lib" { inherit (nixpkgs) lib; };
  home = apps: (hmLib.homeManagerConfiguration {
    pkgs = nixpkgs.legacyPackages.x86_64-linux;
    extraSpecialArgs = { inherit nixpkgs; home-manager = homeManager; };
    modules = [
      (homeManagerModule { apps = { }; home-apps.furnish = { version = 1; inherit apps; }; })
      ./home.nix
    ];
  }).activationPackage;
  userFlake = pkgs.replaceVars ./flake.nix {
    nixpkgs = nixpkgs.outPath;
    furnish = furnish.outPath;
    homeManager = homeManager.outPath;
  };
}
