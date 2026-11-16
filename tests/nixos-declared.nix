# NixOS VM test: jq from environment.systemPackages shows as installed in furnish, from the NixOS
# configuration, and installing it doesn't add a second copy. Run:
#   nix build -L .#checks.x86_64-linux.nixos-declared
{ pkgs, module }:
pkgs.testers.runNixOSTest {
  name = "furnish-nixos-declared";
  nodes.machine = { pkgs, ... }: {
    imports = [ module ];
    services.furnish.enable = true;
    environment.systemPackages = [ pkgs.jq pkgs.git ];
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    machine.wait_for_unit("multi-user.target")
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "mkdir -p /tmp/org/apps",
        "printf 'metadata: { furnish: { description: JSON, categories: [Test] } }\\n"
        "resources:\\n  - name: jq\\n    type: Furnish/Package\\n    properties: { name: jq, packageManager: nix }\\n'"
        " > /tmp/org/apps/jq.dsc.yaml",
        "cd /tmp/org && git init -q && git add -A && git commit -qm apps",
        "furnish collection add /tmp/org --name org",
    )
    listed = machine.succeed("furnish list")
    print(listed)
    assert any(l.startswith("✓  jq") and "in your NixOS configuration" in l for l in listed.splitlines()), listed
    out = machine.succeed("furnish install jq")
    assert "already installed: it is in your NixOS configuration" in out, out
    machine.fail("nix profile list --profile /nix/var/nix/profiles/furnish | grep jq")
    machine.fail("nix profile list | grep jq")
    out = machine.fail("furnish uninstall jq 2>&1")
    assert "remove jq there" in out, out
  '';
}
