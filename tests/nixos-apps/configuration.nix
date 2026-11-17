# The NixOS configuration the test VMs build from their flake, and the tests build beforehand to
# compare. Only building, not switching: no boot loader or real file systems needed.
{ lib, ... }:
{
  services.furnish.enable = true;
  services.furnish.nix = {
    configuration = "/home/alice/cfg";
    allowedOptions = [ "programs.htop.enable" ];
    rebuild = "boot";
  };
  nixpkgs.hostPlatform = "x86_64-linux";
  boot.loader.grub.enable = false;
  fileSystems."/".device = "/dev/vda";
  documentation.enable = false;
  system.stateVersion = "25.11";
  # The VM's nixpkgs is a path: input without a revision; the test's comes from github:.
  system.nixos.versionSuffix = lib.mkForce "";
  system.nixos.revision = lib.mkForce null;
}
