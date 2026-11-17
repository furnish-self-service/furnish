# NixOS VM test: alice installs an app with a Furnish/NixOS resource through the furnish service.
# furnish writes the app into its local flake, updates furnish/apps in alice's flake.lock and
# builds the system as alice, makes it the system the computer starts next (rebuild = "boot"), and
# commits flake.lock because her repository had no other changes. bob may not have alice's
# configuration rebuilt, an option outside the allowlist is refused, an app from a collection alice
# added herself is refused with a message that names it, and an uninstall with other changes in
# the repository leaves flake.lock uncommitted. Run:
#   nix build -L .#checks.x86_64-linux.nixos-install
{ pkgs, nixpkgs, nixosModule, furnish, module }:
let
  s = import ./nixos-apps/systems.nix { inherit pkgs nixpkgs nixosModule furnish; };
  before = s.system { };
  htop = { packages = [ "hello" ]; options."programs.htop.enable" = true; };
  after = s.system { inherit htop; };
  app = name: properties: pkgs.writeText "${name}.dsc.yaml" (builtins.toJSON {
    metadata.furnish = { description = name; categories = [ "Test" ]; };
    resources = [{ inherit name; type = "Furnish/NixOS"; properties = { inherit name; } // properties; }];
  });
in
pkgs.testers.runNixOSTest {
  name = "furnish-nixos-install";
  nodes.machine = { lib, ... }: {
    imports = [ module ];
    services.furnish = {
      enable = true;
      service.enable = true;
      policy = {
        collections.preset = [{ name = "org"; url = "/srv/org"; }];
        service.allowedGroup = "users";
      };
      # As in alice's configuration (nixos-apps/configuration.nix).
      nix = { configuration = "/home/alice/cfg"; allowedOptions = [ "programs.htop.enable" ]; rebuild = "boot"; };
    };
    users.users.alice.isNormalUser = true;
    users.users.bob.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    virtualisation.additionalPaths = [ before after ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def alice(cmd):
        return machine.succeed(f"su - alice -c 'cd cfg && {cmd}'")

    def apps():
        return json.loads(machine.succeed("cat /var/lib/furnish/apps/apps.json"))["apps"]

    def system():
        return machine.succeed("readlink -f /nix/var/nix/profiles/system").strip()

    machine.wait_for_unit("furnish.service")
    machine.fail("curl -sS --max-time 5 https://github.com")
    # The module created furnish's local flake, empty.
    assert apps() == {}, apps()
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "mkdir -p /srv/org/apps",
        "cp ${app "htop" htop} /srv/org/apps/htop.dsc.yaml",
        "cp ${app "sshd" { options."services.openssh.enable" = true; }} /srv/org/apps/sshd.dsc.yaml",
        "cd /srv/org && git init -q && git add -A && git commit -qm apps",
        "mkdir -p /home/alice/cfg",
        "cp ${s.userFlake} /home/alice/cfg/flake.nix",
        "cp ${./nixos-apps/configuration.nix} /home/alice/cfg/configuration.nix",
        "chmod -R u+w /home/alice/cfg && chown -R alice /home/alice/cfg",
    )
    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")
    out = machine.succeed("su - alice -c 'furnish settings'")
    print(out)
    assert "add to its flake.nix" not in out, out

    # alice installs htop: through the service, which rebuilds as alice and commits flake.lock.
    print(machine.succeed("su - alice -c 'furnish install htop' 2>&1"))
    assert apps() == {"htop": {"packages": ["hello"], "options": {"programs.htop.enable": True}}}, apps()
    assert system() == "${after}", system()
    machine.succeed("${after}/sw/bin/hello", "test -e ${after}/sw/bin/htop")
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    assert alice("git status --porcelain").strip() == ""
    listed = machine.succeed("su - alice -c 'furnish list'")
    print(listed)
    assert any(l.startswith("✓  htop") for l in listed.splitlines()), listed

    # bob may not have alice's configuration rebuilt.
    out = machine.fail("su - bob -c 'furnish uninstall htop' 2>&1")
    assert "belongs to another user" in out, out
    assert "htop" in apps() and system() == "${after}"

    # An option outside services.furnish.nix.allowedOptions: refused, nothing changes.
    out = machine.fail("su - alice -c 'furnish install sshd' 2>&1")
    assert "allowedOptions" in out, out
    assert list(apps()) == ["htop"] and system() == "${after}"

    # From a collection alice added herself: the service installs only preset collections' apps,
    # and the refusal names the collection and what the administrator can do (#22).
    machine.succeed(
        "mkdir -p /srv/mine/apps",
        "cp ${app "mytop" { packages = [ "hello" ]; }} /srv/mine/apps/mytop.dsc.yaml",
        "chown -R alice /srv/mine",
    )
    machine.succeed("su - alice -c 'cd /srv/mine && git init -q && git add -A && git commit -qm apps'")
    machine.succeed("su - alice -c 'furnish collection add /srv/mine'")
    out = machine.fail("su - alice -c 'furnish install mytop' 2>&1")
    assert "The collection 'mine' isn't preset" in out and "turn it on" not in out, out
    assert list(apps()) == ["htop"] and system() == "${after}"

    # Uninstalling with another change in the repository: flake.lock stays uncommitted.
    alice('echo "# a note" >> configuration.nix')
    print(machine.succeed("su - alice -c 'furnish uninstall htop' 2>&1"))
    assert apps() == {}, apps()
    assert system() == "${before}", system()
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    status = alice("git status --porcelain")
    assert sorted(status.split()) == ["M", "M", "configuration.nix", "flake.lock"], status
  '';
}
