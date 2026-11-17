# NixOS VM test: apps from furnish's local flake reach the system through the nested input
# furnish/apps, here by hand (nixos-install.nix: through the furnish service). alice runs
# `nix flake update furnish/apps` in her own folder; it changes only that entry of flake.lock, and
# `nixos-rebuild build` then builds the system the test built beforehand with the same apps. The VM
# is offline and starts with only the two systems in its store: furnish's module keeps the
# sources the configuration needs in the system, also after a garbage collection. Run:
#   nix build -L .#checks.x86_64-linux.nixos-apps
{ pkgs, nixpkgs, nixosModule, furnish }:
let
  s = import ./nixos-apps/systems.nix { inherit pkgs nixpkgs nixosModule furnish; };
  before = s.system { };
  after = s.system { hello.packages = [ "hello" ]; };
in
pkgs.testers.runNixOSTest {
  name = "furnish-nixos-apps";
  nodes.machine = { lib, ... }: {
    users.users.alice.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    # Building takes nothing. Evaluating needs nixpkgs, furnish and furnish's crane, which come
    # with the systems (system.extraDependencies, nixpkgs.flake.source).
    virtualisation.additionalPaths = [ before after ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def alice(cmd):
        return machine.succeed(f"su - alice -c 'cd cfg && {cmd}'")

    def lock():
        return json.loads(alice("cat flake.lock"))

    def rebuild():
        alice("nixos-rebuild build --flake .#machine >&2")
        return alice("readlink result").strip()

    def install(apps):
        """What furnish does: write apps.json in its local flake (root's)."""
        doc = json.dumps({"version": 1, "apps": apps})
        machine.succeed(f"echo '{doc}' > /var/lib/furnish/apps/apps.json")

    machine.wait_for_unit("multi-user.target")
    machine.fail("curl -sS --max-time 5 https://github.com")
    machine.succeed("mkdir -p /var/lib/furnish/apps", "cp ${./nixos-apps/apps-flake.nix} /var/lib/furnish/apps/flake.nix")
    install({})
    machine.succeed(
        "mkdir -p /home/alice/cfg",
        "cp ${s.userFlake} /home/alice/cfg/flake.nix",
        "cp ${./nixos-apps/configuration.nix} /home/alice/cfg/configuration.nix",
        "chmod -R u+w /home/alice/cfg && chown -R alice /home/alice/cfg",
    )
    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")
    assert rebuild() == "${before}"

    # Installing hello: only the furnish/apps entry of flake.lock changes.
    old = lock()
    install({"hello": {"packages": ["hello"], "options": {}}})
    alice("nix flake update furnish/apps >&2")
    new = lock()
    node = new["nodes"]["furnish"]["inputs"]["apps"]
    assert new["nodes"][node]["locked"]["narHash"] != old["nodes"][node]["locked"]["narHash"], new
    del old["nodes"][node]["locked"], new["nodes"][node]["locked"]
    assert old == new, (old, new)
    status = alice("git status --porcelain --untracked-files=no")
    assert status.strip() == "M flake.lock", status

    # nixos-rebuild builds the system with hello.
    assert rebuild() == "${after}"
    alice("result/sw/bin/hello")

    # Updating the whole flake keeps furnish/apps pointed at the local flake.
    alice("nix flake update >&2")
    new = lock()
    assert new["nodes"][node]["original"] == {"type": "path", "path": "/var/lib/furnish/apps"}, new
    assert rebuild() == "${after}"

    # After a garbage collection (the system without hello goes), the configuration still
    # evaluates offline: the system keeps furnish's and crane's sources.
    machine.succeed("nix-collect-garbage >&2")
    machine.fail("nix-store --check-validity ${before}")
    machine.succeed("nix-store --check-validity ${furnish.outPath} ${furnish.inputs.crane.outPath} ${nixpkgs.outPath}")
    alice("rm result")
    assert rebuild() == "${after}"
  '';
}
