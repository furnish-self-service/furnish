# The same portable package app in the system and Home Manager installation tests.
{ pkgs, perUser ? false }:
pkgs.writeText "greeting.dsc.yaml" (builtins.toJSON {
  metadata.furnish = {
    description = "Portable greeting command";
    categories = [ "Test" ];
  } // pkgs.lib.optionalAttrs perUser { inherit perUser; };
  resources = [{
    name = "Greeting";
    type = "Furnish/Package";
    condition = "[equals(context().os.family, 'Linux')]";
    properties = {
      name = "portable-greeting";
      namesByManager.nix = "hello";
      command = "hello";
    };
  }];
})
