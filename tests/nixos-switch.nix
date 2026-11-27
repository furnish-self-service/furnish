# NixOS VM test: the furnish service switches to the system it built (rebuild = "switch"), and the
# switch would restart furnish.service, whose definition changed. A switch must not stop the
# service while it runs one: alice's install finishes and commits flake.lock, and the service
# restarts once it has answered. An older furnish.service, which a switch still stops, can't stop
# the switch: it gets to the end and starts the service again. Only root may ask the service to
# restart. The switch is a stand-in (nixos-apps/stand-in-switch.nix). Run:
#   nix build -L .#checks.x86_64-linux.nixos-switch
{ pkgs, nixpkgs, nixosModule, furnish, module }:
let
  s = import ./nixos-apps/systems.nix { inherit pkgs nixpkgs nixosModule furnish; };
  standIn = ./nixos-apps/stand-in-switch.nix;
  before = s.systemWith [ standIn ] { };
  greeting = s.systemWith [ standIn ] { greeting.packages = [ "hello" ]; };
in
pkgs.testers.runNixOSTest {
  name = "furnish-nixos-switch";
  nodes.machine = { lib, ... }: {
    imports = [ module ];
    services.furnish = {
      enable = true;
      service.enable = true;
      policy = {
        collections.preset = [{ name = "org"; url = "/srv/org"; }];
        service.allowedGroup = "users";
      };
      nix = { configuration = "/home/alice/cfg"; rebuild = "switch"; };
    };
    users.users.alice.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    systemd.tmpfiles.rules = [ "d /run/furnish-test 0755 root root" ];
    virtualisation.additionalPaths = [ before greeting ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    def alice(cmd):
        return machine.succeed(f"su - alice -c 'cd cfg && {cmd}'")

    def system():
        return machine.succeed("readlink -f /nix/var/nix/profiles/system").strip()

    def invocation():
        return machine.succeed("systemctl show -p InvocationID --value furnish").strip()

    def restarted_since(old):
        machine.wait_until_succeeds(f"test \"$(systemctl show -p InvocationID --value furnish)\" != {old}", timeout=30)
        machine.wait_until_succeeds("test -S /run/furnish.sock && systemctl is-active furnish", timeout=30)

    machine.wait_for_unit("furnish.service")
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "mkdir -p /srv/org/apps",
        "cp ${import ./nixos-apps/ordinary-package.nix { inherit pkgs; }} /srv/org/apps/greeting.dsc.yaml",
        "cd /srv/org && git init -q && git add -A && git commit -qm apps",
        "mkdir -p /home/alice/cfg",
        "cp ${s.userFlake} /home/alice/cfg/flake.nix",
        "cp ${./nixos-apps/configuration.nix} /home/alice/cfg/base.nix",
        "cp ${standIn} /home/alice/cfg/stand-in-switch.nix",
        "echo '{ imports = [ ./base.nix ./stand-in-switch.nix ]; }' > /home/alice/cfg/configuration.nix",
        "chmod -R u+w /home/alice/cfg && chown -R alice /home/alice/cfg",
    )
    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")

    # The switch leaves the service running: the install finishes, and the service restarts
    # with the new system once it has answered.
    old = invocation()
    print(machine.succeed("su - alice -c 'furnish install greeting' 2>&1"))
    assert system() == "${greeting}", system()
    assert machine.succeed("cat /run/furnish-test/switched").strip() == "switch"
    assert alice("git log -1 --format=%s").strip() == "furnish: install hello"
    restarted_since(old)

    # Only root may ask the service to restart.
    old = invocation()
    out = machine.fail("su - alice -c 'furnish service restart' 2>&1")
    assert "only root" in out, out
    assert invocation() == old
    machine.succeed("furnish service restart")
    restarted_since(old)

    # An older furnish.service, which the switch stops: the switch still gets to the end and
    # starts the service again, though the client lost its answer.
    machine.succeed("rm /run/furnish-test/switched && touch /run/furnish-test/stop")
    out = machine.fail("su - alice -c 'furnish uninstall greeting' 2>&1")
    assert "stopped before finishing" in out, out
    assert system() == "${before}", system()
    machine.wait_for_file("/run/furnish-test/switched", timeout=30)
    assert machine.succeed("cat /run/furnish-test/switched").strip() == "switch"
    machine.wait_for_unit("furnish.service")
  '';
}
