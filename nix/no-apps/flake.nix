# The default of furnish's `apps` input: no apps. furnish's local flake in /var/lib/furnish/apps
# has the same outputs, with the apps people installed in apps.json.
{
  outputs = _: { furnish.systemPackages = [ ]; };
}
