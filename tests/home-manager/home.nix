# The Home Manager configuration the test VM builds from alice's flake, and the test builds
# beforehand to compare.
{
  home.username = "alice";
  home.homeDirectory = "/home/alice";
  home.stateVersion = "25.11";
  services.furnish.enable = true;
  services.furnish.nix = {
    configuration = "/home/alice/.config/home-manager";
    allowedOptions = [ "programs.htop.enable" "programs.ssh.includes" ];
  };
  # Home Manager writes ~/.ssh/config, for the app that adds an Include (tests/home-manager.nix).
  programs.ssh.enable = true;
  # An include of her own, which Home Manager writes into the same line as the app's.
  programs.ssh.includes = [ "~/.ssh/own.conf" ];
  manual.manpages.enable = false;
}
