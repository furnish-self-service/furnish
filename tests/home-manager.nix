# NixOS VM test: alice installs apps with a Furnish/NixOS resource into her standalone Home Manager
# configuration. `furnish settings` shows the lines her flake.nix needs and creates furnish's local
# flake in her data folder ($XDG_DATA_HOME, ~/data here), which the furnish/home-apps input points
# at. The organization's apps install through the furnish service, which acts as alice: it finds
# the folder from her flake.lock, writes the app there, updates only that entry of flake.lock,
# builds and activates the generation the test built beforehand with the same apps, and commits
# flake.lock. An option outside her allowlist is refused, an app with only NixOS options needs the
# NixOS configuration, and an uninstall with other changes in the repository leaves flake.lock
# uncommitted. The owner's example from #8, an app for each person (perUser): an ssh Include
# through a Furnish/NixOS step (onlyIfSetUp) before a Furnish/Line step. For alice, Home Manager
# writes it into one Include line with her own include, and the Line step accepts that line; for
# bob, without Home Manager, the first step isn't needed and the Line step writes it. With
# installInto set to system in alice's settings (in ~/data), the service installs into the NixOS
# configuration instead. After a garbage collection, the configuration still builds offline,
# because it keeps nixpkgs and Home Manager (home.extraDependencies). The VM is offline. Run:
#   nix build -L .#checks.x86_64-linux.home-manager
{ pkgs, nixpkgs, homeManagerModule, furnish, module }:
let
  h = import ./home-manager/homes.nix { inherit pkgs nixpkgs homeManagerModule furnish; };
  before = h.home { };
  htop = { packages = [ "hello" ]; options."programs.htop.enable" = true; };
  hello = app "hello" { packages = [ "hello" ]; };
  after = h.home { inherit htop; };
  include = "config.d/acme.conf";
  ssh = h.home { acme-ssh = { packages = [ ]; options."programs.ssh.includes" = [ include ]; }; };
  acme-ssh = pkgs.writeText "acme-ssh.dsc.yaml" (builtins.toJSON {
    metadata.furnish = { description = "acme-ssh"; categories = [ "Test" ]; perUser = true; };
    resources = [
      {
        name = "SSH include through Home Manager";
        type = "Furnish/NixOS";
        properties = { name = "acme-ssh"; onlyIfSetUp = true; homeOptions."programs.ssh.includes" = [ include ]; };
      }
      {
        name = "SSH include";
        type = "Furnish/Line";
        properties = {
          path = "~/.ssh/config";
          line = "Include ${include}";
          before = "^\\s*(Host|Match)\\s";
          accept = "^\\s*Include\\s(.*\\s)?config\\.d/acme\\.conf(\\s|$)";
        };
      }
    ];
  });
  app = name: properties: pkgs.writeText "${name}.dsc.yaml" (builtins.toJSON {
    metadata.furnish = { description = name; categories = [ "Test" ]; };
    resources = [{ inherit name; type = "Furnish/NixOS"; properties = { inherit name; } // properties; }];
  });
in
pkgs.testers.runNixOSTest {
  name = "furnish-home-manager";
  nodes.machine = { lib, ... }: {
    imports = [ module ];
    services.furnish = {
      enable = true;
      service.enable = true;
      policy = {
        collections.preset = [{ name = "org"; url = "/srv/org"; }];
        service.allowedGroup = "users";
      };
    };
    users.users.alice.isNormalUser = true;
    users.users.bob.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    # Building takes nothing. Evaluating needs nixpkgs, Home Manager, furnish and crane.
    virtualisation.additionalPaths = [ before after ssh nixpkgs.outPath h.homeManager.outPath furnish.outPath furnish.inputs.crane.outPath ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json

    def alice(cmd, dir=".config/home-manager"):
        return machine.succeed(f"su - alice -c 'cd {dir} && {cmd}'")

    def furnish(cmd, ok=True):
        run = machine.succeed if ok else machine.fail
        out = run(f"su - alice -c 'furnish {cmd}' 2>&1")
        print(out)
        return out

    def lock():
        return json.loads(alice("cat flake.lock"))

    def apps():
        return json.loads(alice("cat apps.json", dir="data/furnish/home-apps"))["apps"]

    def generation():
        return alice("readlink -f ~/.local/state/nix/profiles/home-manager").strip()

    machine.wait_for_unit("furnish.service")
    machine.fail("curl -sS --max-time 5 https://github.com")
    machine.succeed(
        "git config --global user.name t && git config --global user.email t@t",
        "mkdir -p /srv/org/apps",
        "cp ${app "htop" { inherit (htop) packages; homeOptions = htop.options; }} /srv/org/apps/htop.dsc.yaml",
        "cp ${app "git" { homeOptions."programs.git.enable" = true; }} /srv/org/apps/git.dsc.yaml",
        "cp ${app "sshd" { options."services.openssh.enable" = true; }} /srv/org/apps/sshd.dsc.yaml",
        "cp ${acme-ssh} /srv/org/apps/acme-ssh.dsc.yaml",
        "cp ${hello} /srv/org/apps/hello.dsc.yaml",
        "cd /srv/org && git init -q && git add -A && git commit -qm apps",
    )
    alice("echo export XDG_DATA_HOME=/home/alice/data > .profile && mkdir -p .config/home-manager", dir=".")
    alice("cp ${h.userFlake} flake.nix && cp ${./home-manager/home.nix} home.nix && chmod u+w *")

    # Before the module is on: furnish shows the lines and creates its local flake.
    out = furnish("settings")
    assert 'inputs.furnish.inputs.home-apps.url = "path:/home/alice/data/furnish/home-apps";' in out, out
    assert "homeManagerModules.default" in out, out
    assert apps() == {}, apps()

    alice("git config --global user.name a && git config --global user.email a@a && git init -q && git add -A")
    alice("nix flake lock >&2 && git add -A && git commit -qm config")
    alice("nix build .#homeConfigurations.alice.activationPackage >&2 && result/activate >&2 && rm result")
    assert generation() == "${before}", generation()
    out = furnish("settings")
    assert "home-apps.url" not in out, out
    assert "Apps go into:        your Home Manager configuration (automatic)" in out, out

    # Installing htop, through the service: only furnish/home-apps changes in flake.lock, the
    # generation has hello and htop, and flake.lock is committed, all as alice.
    old = lock()
    furnish("install htop")
    assert apps() == {"htop": {"packages": ["hello"], "options": {"programs.htop.enable": True}}}, apps()
    assert generation() == "${after}", generation()
    alice("hello && htop --version")
    machine.succeed("stat -c %U /home/alice/data/furnish/home-apps/apps.json | grep -qx alice")
    machine.fail("test -e /home/alice/.local/share/furnish/home-apps")
    new = lock()
    node = new["nodes"]["furnish"]["inputs"]["home-apps"]
    del old["nodes"][node]["locked"], new["nodes"][node]["locked"]
    assert old == new, (old, new)
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    assert alice("git status --porcelain").strip() == ""
    listed = furnish("list")
    assert any(l.startswith("✓  htop") for l in listed.splitlines()), listed

    # An option outside services.furnish.nix.allowedOptions: refused, nothing changes.
    out = furnish("install git", ok=False)
    assert "allowedOptions" in out, out
    assert list(apps()) == ["htop"] and generation() == "${after}"

    # Only NixOS options: the app goes into the NixOS configuration, which isn't set up here.
    out = furnish("install sshd", ok=False)
    assert "NixOS" in out, out
    assert list(apps()) == ["htop"] and generation() == "${after}"

    # Uninstalling with another change in the repository: flake.lock stays uncommitted.
    alice('echo "# a note" >> home.nix')
    furnish("uninstall htop")
    assert apps() == {}, apps()
    assert generation() == "${before}", generation()
    alice("! command -v hello")
    assert alice("git log -1 --format=%s").strip() == "furnish: install htop"
    status = alice("git status --porcelain")
    assert sorted(status.split()) == ["M", "M", "flake.lock", "home.nix"], status

    # The ssh Include: Home Manager writes it into ~/.ssh/config (a link into the store), in one
    # line with alice's own include, and the Line step, which runs after the activation, accepts it.
    furnish("install acme-ssh")
    assert generation() == "${ssh}", generation()
    machine.succeed("test -L /home/alice/.ssh/config")
    config = machine.succeed("cat /home/alice/.ssh/config")
    print(config)
    assert any(l.startswith("Include ") and "~/.ssh/own.conf" in l.split() and "${include}" in l.split() for l in config.splitlines()), config
    listed = furnish("list")
    assert any(l.startswith("✓  acme-ssh") for l in listed.splitlines()), listed

    # bob has no Home Manager: the first step isn't needed, the Line step writes the file.
    machine.succeed("su - bob -c 'git config --global --add safe.directory /srv/org/.git'")
    out = machine.succeed("su - bob -c 'furnish install acme-ssh --dry-run' 2>&1")
    print(out)
    assert "Not needed here: Home Manager isn't set up for you" in out, out
    print(machine.succeed("su - bob -c 'furnish install acme-ssh' 2>&1"))
    machine.succeed("test ! -L /home/bob/.ssh/config", "grep -qx 'Include ${include}' /home/bob/.ssh/config")
    machine.fail("test -e /home/bob/.local/share/furnish/home-apps/apps.json")
    listed = machine.succeed("su - bob -c 'furnish list'")
    assert any(l.startswith("✓  acme-ssh") for l in listed.splitlines()), listed

    machine.succeed("su - bob -c 'echo HashKnownHosts yes >> ~/.ssh/config'")
    out = machine.succeed("su - bob -c 'furnish uninstall acme-ssh --dry-run' 2>&1")
    assert "- remove         SSH include " in out, out
    machine.succeed("su - bob -c 'furnish uninstall acme-ssh' 2>&1")
    assert machine.succeed("cat /home/bob/.ssh/config").strip() == "HashKnownHosts yes"

    # alice wants apps in the NixOS configuration: her setting is in ~/data, which the service
    # can't see; furnish sends it with the request. A packages-only app then goes there, which
    # isn't set up on this computer.
    furnish("settings --nixos-install-into system")
    machine.succeed("grep -q installInto /home/alice/data/furnish/settings.json")
    out = furnish("settings")
    assert "Apps go into:        the NixOS configuration (set by you)" in out, out
    out = furnish("install hello", ok=False)
    assert "NixOS configuration" in out and "Home Manager" not in out, out
    assert "hello" not in apps() and generation() == "${ssh}"
    furnish("settings --nixos-reset")

    # After a garbage collection, alice's configuration still builds offline: the generation
    # keeps furnish's, crane's, nixpkgs' and Home Manager's sources.
    machine.succeed("nix-collect-garbage >&2")
    machine.succeed("nix-store --check-validity ${nixpkgs.outPath} ${h.homeManager.outPath} ${furnish.outPath}")
    alice("nix build .#homeConfigurations.alice.activationPackage >&2")
    assert alice("readlink result").strip() == "${ssh}"

    # Removing the app from a combined Include keeps alice's own entry after activation.
    out = furnish("uninstall acme-ssh --dry-run")
    assert "= nothing to do  SSH include " in out, out
    furnish("uninstall acme-ssh")
    config = machine.succeed("cat /home/alice/.ssh/config")
    assert "~/.ssh/own.conf" in config and "${include}" not in config, config
  '';
}
