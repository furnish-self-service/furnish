# NixOS VM test: apps from furnish's local flake in alice's data folder reach her standalone
# Home Manager configuration through the nested input furnish/home-apps, here by hand. alice runs
# `nix flake update furnish/home-apps` in her configuration; it changes only that entry of
# flake.lock, and building and activating the configuration (what `home-manager switch` does, without
# its news, which would need a build) gives the generation the test built beforehand with the same
# apps. Everything runs as alice. The VM is offline. Run:
#   nix build -L .#checks.x86_64-linux.home-manager
{ pkgs, nixpkgs, homeManagerModule, furnish }:
let
  h = import ./home-manager/homes.nix { inherit pkgs nixpkgs homeManagerModule furnish; };
  before = h.home { };
  after = h.home { hello.packages = [ "hello" ]; hello.options."programs.htop.enable" = true; };
in
pkgs.testers.runNixOSTest {
  name = "furnish-home-manager";
  nodes.machine = { lib, ... }: {
    users.users.alice.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    # Building takes nothing. Evaluating needs nixpkgs, Home Manager, furnish and crane.
    virtualisation.additionalPaths = [ before after nixpkgs.outPath h.homeManager furnish.outPath furnish.inputs.crane.outPath ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def alice(cmd, dir=".config/home-manager"):
        return machine.succeed(f"su - alice -c 'cd {dir} && {cmd}'")

    def lock():
        return json.loads(alice("cat flake.lock"))

    def build():
        alice("nix build .#homeConfigurations.alice.activationPackage >&2")
        return alice("readlink result").strip()

    def install(apps):
        """What furnish does: write apps.json in alice's local flake."""
        doc = json.dumps({"version": 1, "apps": apps})
        path = "/home/alice/.local/share/furnish/home-apps/apps.json"
        machine.succeed(f"echo '{doc}' > {path} && chown alice {path}")

    machine.wait_for_unit("multi-user.target")
    machine.fail("curl -sS --max-time 5 https://github.com")
    alice("mkdir -p .local/share/furnish/home-apps .config/home-manager", dir=".")
    alice("cp ${./nixos-apps/apps-flake.nix} flake.nix", dir=".local/share/furnish/home-apps")
    install({})
    alice("cp ${h.userFlake} flake.nix && cp ${./home-manager/home.nix} home.nix && chmod u+w *")
    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")
    assert build() == "${before}"

    # Installing hello: only the furnish/home-apps entry of flake.lock changes.
    old = lock()
    install({"hello": {"packages": ["hello"], "options": {"programs.htop.enable": True}}})
    alice("nix flake update furnish/home-apps >&2")
    new = lock()
    node = new["nodes"]["furnish"]["inputs"]["home-apps"]
    assert new["nodes"][node]["locked"]["narHash"] != old["nodes"][node]["locked"]["narHash"], new
    del old["nodes"][node]["locked"], new["nodes"][node]["locked"]
    assert old == new, (old, new)
    status = alice("git status --porcelain --untracked-files=no")
    assert status.strip() == "M flake.lock", status

    # Activating it, as `home-manager switch` does, as alice: the generation has hello and htop.
    assert build() == "${after}"
    alice("result/activate >&2")
    generation = alice("readlink -f ~/.local/state/nix/profiles/home-manager").strip()
    assert generation == "${after}", generation
    alice("hello && htop --version")
    facts = json.loads(alice("cat ~/.config/furnish/home-manager.json"))
    assert facts["nix"]["configuration"] == "/home/alice/.config/home-manager", facts
    assert facts["nixosModule"] is False, facts

    # Updating the whole flake keeps furnish/home-apps pointed at the local flake.
    alice("nix flake update >&2")
    new = lock()
    assert new["nodes"][node]["original"] == {"type": "path", "path": "/home/alice/.local/share/furnish/home-apps"}, new
    assert build() == "${after}"

    # After a garbage collection, the generation keeps furnish's and crane's sources. Building
    # offline also needs nixpkgs' and Home Manager's, which standalone Home Manager doesn't keep.
    alice("rm result")
    machine.succeed("nix-collect-garbage >&2")
    machine.fail("nix-store --check-validity ${before}")
    machine.succeed("nix-store --check-validity ${furnish.outPath} ${furnish.inputs.crane.outPath}")
  '';
}
