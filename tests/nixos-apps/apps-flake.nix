# furnish's local apps flake (/var/lib/furnish/apps), as furnish writes it: the apps are in apps.json.
{
  outputs = _: { furnish = builtins.fromJSON (builtins.readFile ./apps.json); };
}
