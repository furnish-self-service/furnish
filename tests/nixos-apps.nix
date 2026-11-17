# NixOS VM test: apps from furnish's local flake reach the system through the nested input
# furnish/apps. `nix flake update furnish/apps` changes only that entry of flake.lock, and a plain
# `nixos-rebuild build` then builds the system the test built beforehand with the same apps. The VM
# is offline, so it also shows what Nix needs locally. Run:
#   nix build -L .#checks.x86_64-linux.nixos-apps
{ pkgs, nixpkgs, nixosModule, furnish }:
let
  # The system the VM's configuration should build, with these apps.
  system = packages: (nixpkgs.lib.nixosSystem {
    modules = [ (nixosModule { furnish.systemPackages = packages; }) ./nixos-apps/configuration.nix ];
  }).config.system.build.toplevel;
  before = system [ ];
  after = system [ "hello" ];
  userFlake = pkgs.replaceVars ./nixos-apps/flake.nix { nixpkgs = nixpkgs.outPath; furnish = furnish.outPath; };
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
    # Both systems are there, so building them takes nothing. Evaluating needs the sources of
    # nixpkgs, furnish and furnish's crane (furnish/crane in flake.lock): the VM is offline.
    virtualisation.additionalPaths = [ before after nixpkgs.outPath furnish.outPath furnish.inputs.crane.outPath ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def lock():
        return json.loads(machine.succeed("cat /home/alice/cfg/flake.lock"))

    def rebuild():
        machine.succeed("cd /home/alice/cfg && nixos-rebuild build --flake .#machine >&2")
        return machine.succeed("readlink /home/alice/cfg/result").strip()

    def install(apps):
        """What furnish does: write apps.json in its local flake and commit it."""
        machine.succeed(
            f"echo '{json.dumps({'systemPackages': apps})}' > /var/lib/furnish/apps/apps.json",
            "cd /var/lib/furnish/apps && git add -A && git commit -qm apps",
        )

    machine.wait_for_unit("multi-user.target")
    machine.fail("curl -sS --max-time 5 https://github.com")
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "git config --global init.defaultBranch main",
        "mkdir -p /var/lib/furnish/apps",
        "cp ${./nixos-apps/apps-flake.nix} /var/lib/furnish/apps/flake.nix",
        "git -C /var/lib/furnish/apps init -q",
    )
    install([])
    # alice's configuration, in git as usual. Nix (libgit2) refuses a repository that belongs to
    # someone else unless git's safe.directory names it, also for root.
    machine.succeed(
        "mkdir -p /home/alice/cfg",
        "cp ${userFlake} /home/alice/cfg/flake.nix",
        "cp ${./nixos-apps/configuration.nix} /home/alice/cfg/configuration.nix",
        "chmod -R u+w /home/alice/cfg && chown -R alice /home/alice/cfg",
        "su - alice -c 'git config --global user.name a && git config --global user.email a@a'",
        "su - alice -c 'git config --global init.defaultBranch main && cd cfg && git init -q && git add -A'",
    )
    out = machine.fail("cd /home/alice/cfg && nix flake lock 2>&1")
    assert "is not owned by current user" in out, out
    machine.succeed(
        "git config --global --add safe.directory /home/alice/cfg",
        "cd /home/alice/cfg && nix flake lock >&2",
        "chown alice /home/alice/cfg/flake.lock",
        "su - alice -c 'cd cfg && git add -A && git commit -qm config'",
    )
    assert rebuild() == "${before}"

    # Installing hello: only the furnish/apps entry of flake.lock changes.
    old = lock()
    install(["hello"])
    machine.succeed("cd /home/alice/cfg && nix flake update furnish/apps >&2")
    new = lock()
    node = new["nodes"]["furnish"]["inputs"]["apps"]
    assert new["nodes"][node]["locked"]["rev"] != old["nodes"][node]["locked"]["rev"], new
    del old["nodes"][node]["locked"], new["nodes"][node]["locked"]
    assert old == new, (old, new)
    status = machine.succeed("git -C /home/alice/cfg status --porcelain --untracked-files=no")
    assert status.strip() == "M flake.lock", status

    # A plain nixos-rebuild builds the system with hello.
    assert rebuild() == "${after}"
    machine.succeed("/home/alice/cfg/result/sw/bin/hello")

    # Updating the whole flake keeps furnish/apps pointed at the local flake.
    machine.succeed("cd /home/alice/cfg && nix flake update >&2")
    new = lock()
    print(json.dumps(new, indent=2))
    assert new["nodes"][node]["original"] == {"type": "git", "url": "file:///var/lib/furnish/apps"}, new
    assert rebuild() == "${after}"

    # Without follows, furnish evaluates with its own nixpkgs from its flake.lock. Offline that
    # works only because it is the same nixpkgs, already in the store.
    machine.succeed(
        "mkdir /tmp/nf && cp /home/alice/cfg/*.nix /tmp/nf && sed -i /follows/d /tmp/nf/flake.nix",
        "cd /tmp/nf && git init -q && git add -A && nix flake lock >&2",
    )
    out = machine.succeed("cd /tmp/nf && nix eval --raw .#nixosConfigurations.machine.config.system.build.toplevel")
    assert out == "${after}", out

    # alice herself can't update furnish/apps: the local flake belongs to root. A path: input has
    # no such check.
    out = machine.fail("su - alice -c 'cd cfg && nix flake update furnish/apps' 2>&1")
    assert "is not owned by current user" in out, out
    machine.succeed(
        "cp -r /home/alice/cfg /tmp/pa && rm -rf /tmp/pa/.git /tmp/pa/flake.lock /tmp/pa/result",
        "sed -i s,git+file:,path:, /tmp/pa/flake.nix && chown -R alice /tmp/pa",
        "su - alice -c 'cd /tmp/pa && git init -q && git add -A && nix flake lock' >&2",
    )
    out = machine.succeed("su - alice -c 'cd /tmp/pa && nix eval --raw .#nixosConfigurations.machine.config.system.build.toplevel'")
    assert out == "${after}", out
  '';
}
