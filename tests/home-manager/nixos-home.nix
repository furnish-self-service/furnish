# Home Manager as a NixOS module, with furnish's Home Manager module on for alice. Shared by the
# configuration in alice's folder and the test VM itself.
{
  users.users.alice.isNormalUser = true;
  home-manager.users.alice = {
    home.stateVersion = "25.11";
    manual.manpages.enable = false;
    services.furnish.enable = true;
    services.furnish.nix.allowedOptions = [ "programs.htop.enable" ];
  };
}
