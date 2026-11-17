# NixOS VM test: jq from environment.systemPackages shows as installed in furnish, from the NixOS
# configuration, and installing it doesn't add a second copy. furnish finds the configuration
# through /etc/nixos/flake.nix linked to /home/alice/cfg, and then where the module's
# services.furnish.nix options put it. Run:
#   nix build -L .#checks.x86_64-linux.nixos-declared
{ pkgs, module }:
pkgs.testers.runNixOSTest {
  name = "furnish-nixos-declared";
  nodes.machine = { pkgs, ... }: {
    imports = [ module ];
    services.furnish.enable = true;
    # Not there at first: furnish ignores it and looks in /etc/nixos.
    services.furnish.nix = { configuration = "/etc/fleet"; flakeAttribute = "fleet"; };
    users.users.alice.isNormalUser = true;
    environment.systemPackages = [ pkgs.jq pkgs.git ];
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    machine.wait_for_unit("multi-user.target")
    flake = "{ outputs = _: { nixosConfigurations.%s = { }; }; }"
    machine.succeed(
        "mkdir -p /home/alice/cfg /etc/nixos",
        f"echo '{flake % 'fleet'}' > /home/alice/cfg/flake.nix",
        "chown -R alice /home/alice/cfg",
        "ln -sfn /home/alice/cfg/flake.nix /etc/nixos/flake.nix",
    )
    out = machine.succeed("furnish settings")
    print(out)
    assert "NixOS configuration: /home/alice/cfg (flake, from /etc/nixos)" in out, out
    assert "ignoring /etc/fleet" in out, out
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
    # The resource knows the configuration's folder from furnish (FURNISH_DETECTED__*).
    assert any(l.startswith("✓  jq") and "in your NixOS configuration (/home/alice/cfg)" in l for l in listed.splitlines()), listed
    out = machine.succeed("furnish install jq")
    assert "already installed: it is in your NixOS configuration" in out, out
    machine.fail("nix profile list --profile /nix/var/nix/profiles/furnish | grep jq")
    machine.fail("nix profile list | grep jq")
    out = machine.fail("furnish uninstall jq 2>&1")
    assert "remove jq there" in out, out

    # Where the module puts it, once it exists: the organization's choice, locked.
    machine.succeed("mkdir -p /etc/fleet", f"echo '{flake % 'fleet'}' > /etc/fleet/flake.nix")
    out = machine.succeed("furnish settings")
    print(out)
    assert "NixOS configuration: /etc/fleet (flake, set in the NixOS configuration, services.furnish.nix)" in out, out
    assert "NixOS flake name:    fleet (set in the NixOS configuration)" in out, out
    out = machine.fail("furnish settings --nixos-configuration /home/alice/cfg 2>&1")
    assert "set by your organization" in out, out
    policy = machine.succeed("cat /etc/furnish/policy.json")
    assert '"locked":true' in policy and '"/etc/fleet"' in policy, policy
    facts = machine.succeed("cat /etc/furnish/nixos.json")
    assert '"hostName":"machine"' in facts and '"configuration":"/etc/fleet"' in facts, facts
  '';
}
