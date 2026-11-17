# The default of furnish's `apps` input: no apps. furnish's local flake in /var/lib/furnish/apps
# has the same output, with the apps furnish installed (Furnish/NixOS) in apps.json.
{
  outputs = _: { furnish = { version = 1; apps = { }; }; };
}
