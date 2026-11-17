# The Home Manager configuration the test VM builds from alice's flake, and the test builds
# beforehand to compare.
{
  home.username = "alice";
  home.homeDirectory = "/home/alice";
  home.stateVersion = "25.11";
  services.furnish.enable = true;
  services.furnish.nix = {
    configuration = "/home/alice/.config/home-manager";
    allowedOptions = [ "programs.htop.enable" ];
  };
  manual.manpages.enable = false;
}
