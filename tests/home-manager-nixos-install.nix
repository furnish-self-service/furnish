# NixOS VM test: Home Manager as a NixOS module. alice installs an app with a Furnish/NixOS
# resource through the furnish service; it goes into her Home Manager configuration, because
# furnish's Home Manager module is on for her. The service writes it under homes.alice in the
# system's apps flake, updates furnish/apps in alice's flake.lock and builds the system as alice,
# makes it the system the computer starts next (rebuild = "boot") and commits flake.lock. An
# option outside her allowlist is refused, and an uninstall with other changes in the repository
# leaves flake.lock uncommitted. Switching to the built system would replace the VM's own
# configuration, so the test compares it with the system built beforehand instead. Run:
#   nix build -L .#checks.x86_64-linux.home-manager-nixos-install
{ pkgs, nixpkgs, nixosModule, furnish, module }:
let
  h = import ./home-manager/homes.nix { inherit pkgs nixpkgs furnish; homeManagerModule = null; };
  s = import ./home-manager/nixos-systems.nix { inherit pkgs nixpkgs nixosModule furnish; inherit (h) homeManager; };
  htop = { packages = [ "hello" ]; options."programs.htop.enable" = true; };
  before = s.system { };
  after = s.system { homes.alice.apps = { inherit htop; }; };
  app = name: properties: pkgs.writeText "${name}.dsc.yaml" (builtins.toJSON {
    metadata.furnish = { description = name; categories = [ "Test" ]; perUser = true; };
    resources = [
      { inherit name; type = "Furnish/NixOS"; properties = { inherit name; onlyIfSetUp = true; } // properties; }
      { name = "User settings"; type = "Furnish/File"; properties = { path = "~/.furnish-test"; content = "managed for this person"; }; }
    ];
  });
in
pkgs.testers.runNixOSTest {
  name = "furnish-home-manager-nixos-install";
  nodes.machine = { lib, ... }: {
    imports = [ module "${h.homeManager}/nixos" ./home-manager/nixos-home.nix ];
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
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    virtualisation.additionalPaths = [ before after h.homeManager.outPath ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def alice(cmd):
        return machine.succeed(f"su - alice -c 'cd cfg && {cmd}'")

    def furnish(cmd, ok=True):
        run = machine.succeed if ok else machine.fail
        out = run(f"su - alice -c 'furnish {cmd}' 2>&1")
        print(out)
        return out

    def apps():
        return json.loads(machine.succeed("cat /var/lib/furnish/apps/apps.json"))

    def system():
        return machine.succeed("readlink -f /nix/var/nix/profiles/system").strip()

    machine.wait_for_unit("furnish.service")
    # Home Manager activated her generation at boot (a unit that exits when done).
    machine.wait_for_file("/home/alice/.config/furnish/home-manager.json")
    machine.fail("curl -sS --max-time 5 https://github.com")
    facts = json.loads(machine.succeed("cat /home/alice/.config/furnish/home-manager.json"))
    assert facts["nixosModule"] is True, facts
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "mkdir -p /srv/org/apps",
        "cp ${app "htop" { inherit (htop) packages; homeOptions = htop.options; }} /srv/org/apps/htop.dsc.yaml",
        "cp ${app "git" { homeOptions."programs.git.enable" = true; }} /srv/org/apps/git.dsc.yaml",
        "cd /srv/org && git init -q && git add -A && git commit -qm apps",
        "mkdir -p /home/alice/cfg",
        "cp ${s.userFlake} /home/alice/cfg/flake.nix",
        "cp ${./nixos-apps/configuration.nix} /home/alice/cfg/configuration.nix",
        "cp ${./home-manager/nixos-home.nix} /home/alice/cfg/nixos-home.nix",
        "chmod -R u+w /home/alice/cfg && chown -R alice /home/alice/cfg",
    )
    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")
    out = furnish("settings")
    assert "Apps go into:        your Home Manager configuration (automatic)" in out, out
    assert "add to" not in out, out

    # alice installs htop: into her part of the system's apps flake; the system has it in her
    # Home Manager generation only.
    preview = furnish("install htop --dry-run")
    assert "Home Manager" in preview and "User settings" in preview, preview
    machine.fail("test -e /home/alice/.furnish-test")
    furnish("install htop")
    machine.succeed("test $(stat -c %U /home/alice/.furnish-test) = alice")
    machine.fail("test -e /root/.furnish-test")
    assert apps() == {"version": 1, "apps": {}, "homes": {"alice": {"apps": {"htop": {"packages": ["hello"], "options": {"programs.htop.enable": True}}}}}}, apps()
    assert system() == "${after}", system()
    machine.succeed("nix-store -qR ${after} | grep -q -- -hello-")
    machine.fail("test -e ${after}/sw/bin/hello")
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    assert alice("git status --porcelain").strip() == ""

    # An option outside alice's allowlist in the NixOS configuration: refused, nothing changes.
    out = furnish("install git", ok=False)
    assert "allowedOptions" in out, out
    assert list(apps()["homes"]["alice"]["apps"]) == ["htop"] and system() == "${after}"

    # Uninstalling with another change in the repository: flake.lock stays uncommitted.
    alice('echo "# a note" >> configuration.nix')
    furnish("uninstall htop")
    machine.fail("test -e /home/alice/.furnish-test")
    assert apps() == {"version": 1, "apps": {}}, apps()
    assert system() == "${before}", system()
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    status = alice("git status --porcelain")
    assert sorted(status.split()) == ["M", "M", "configuration.nix", "flake.lock"], status
  '';
}
