# NixOS VM test: alice installs an app with a Furnish/NixOS resource through the furnish service.
# furnish writes the app into its local flake, updates furnish/apps in alice's flake.lock and
# builds the system as alice, makes it the system the computer starts next (rebuild = "boot"), and
# commits flake.lock because her repository had no other changes. bob may not have alice's
# configuration rebuilt, an invalid option name is refused, an app from a collection alice
# added herself is refused with a message that names it, and an uninstall with other changes in
# the repository leaves flake.lock uncommitted. A configuration in /etc/nixos that wheel can change
# (root:wheel 2775) is rebuilt for alice, a member of wheel, which the module trusts because sudo
# lets wheel become root; another group that can change it is refused. As an admin's clone of it
# would be, its files and git folder belong to alice and only the folder itself to root (#24):
# root's git and Nix use it because alice is in wheel, and refuse it when the git folder belongs
# to bob, who isn't. Run:
#   nix build -L .#checks.x86_64-linux.nixos-install
{ pkgs, nixpkgs, nixosModule, furnish, module }:
let
  s = import ./nixos-apps/systems.nix { inherit pkgs nixpkgs nixosModule furnish; };
  before = s.system { };
  greeting = s.system { greeting.packages = [ "hello" ]; };
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
      nix = { configuration = "/home/alice/cfg"; rebuild = "boot"; };
    };
    users.users.alice = { isNormalUser = true; extraGroups = [ "wheel" ]; };
    users.users.bob.isNormalUser = true;
    environment.systemPackages = [ pkgs.git ];
    nix.settings = {
      experimental-features = [ "nix-command" "flakes" ];
      substituters = lib.mkForce [ ];
    };
    virtualisation.additionalPaths = [ before after greeting ];
    virtualisation.memorySize = 4096;
  };
  testScript = ''
    import json
    import shlex

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
        "cp ${import ./nixos-apps/ordinary-package.nix { inherit pkgs; }} /srv/org/apps/greeting.dsc.yaml",
        "cp ${app "preconfigured" { options = { "boot.loader.grub.enable" = false; "system.stateVersion" = "25.11"; }; }} /srv/org/apps/preconfigured.dsc.yaml",
        "cp ${app "htop" htop} /srv/org/apps/htop.dsc.yaml",
        "cp ${app "invalid" { options."bad option" = true; }} /srv/org/apps/invalid.dsc.yaml",
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

    # Already in the evaluated flake: status and preview agree, install does not claim the
    # options, and removal leaves the source configuration and generation untouched.
    initial = (system(), alice("git rev-parse HEAD"))
    listing = alice("furnish list")
    assert any(line.startswith("✓  preconfigured") and "in your NixOS configuration" in line for line in listing.splitlines()), listing
    preview = alice("furnish install preconfigured --dry-run")
    assert "Add preconfigured" not in preview and "rebuild it" not in preview, preview
    alice("furnish install preconfigured")
    out = machine.fail("su - alice -c 'furnish uninstall preconfigured' 2>&1")
    assert "in your NixOS configuration" in out and "boot.loader.grub.enable" in out, out
    assert apps() == {} and (system(), alice("git rev-parse HEAD")) == initial
    assert alice("git status --porcelain").strip() == ""

    # An ordinary package selected Nix and resolved its nixpkgs name through namesByManager.
    # rebuild=boot leaves the running system alone; inspect the generation selected for next boot.
    link_paths = "readlink ~/.nix-profile /nix/var/nix/profiles/furnish || true"
    profile_paths = "readlink -e ~/.nix-profile ~/.local/state/nix/profiles/profile /nix/var/nix/profiles/furnish || true"
    links = alice(link_paths)
    generations = alice(profile_paths)
    preview = alice("furnish install greeting --dry-run")
    assert "NixOS" in preview and "nix profile install" not in preview, preview
    assert apps() == {}, apps()
    after_preview = (alice(link_paths), alice(profile_paths))
    assert after_preview == (links, generations), ((links, generations), after_preview)
    alice("furnish install greeting")
    assert list(apps().values()) == [{"packages": ["hello"], "options": {}}], apps()
    assert system() == "${greeting}", system()
    assert alice("git log -1 --format=%s").strip() == "furnish: install hello"
    machine.succeed("${greeting}/sw/bin/hello")
    # A Nix flake build may initialize a compatibility symlink. Only existing targets are
    # installed profile generations; they must remain exactly as they were before the rebuild.
    after_install = alice(profile_paths)
    assert after_install == generations, (generations, after_install)
    alice("test ! -e ~/.nix-profile/bin/hello && test ! -e /nix/var/nix/profiles/furnish/bin/hello")
    assert any(line.startswith("✓  greeting") for line in alice("furnish list").splitlines())
    alice("furnish uninstall greeting")
    assert apps() == {} and system() == "${before}", apps()

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

    # Option names are validated before changing the configuration.
    out = machine.fail("su - alice -c 'furnish install invalid' 2>&1")
    assert "bad option" in out, out
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

    # A configuration the admins share (#21): /etc/nixos as root:wheel 2775. sudo lets wheel run
    # any command as root, so the module trusts wheel, and alice, a member, installs into it. Its
    # files and git folder are alice's, as after she cloned it (#24): root's git and Nix refuse
    # such a repository unless furnish allows it with safe.directory.
    facts = json.loads(machine.succeed("cat /etc/furnish/nixos.json"))
    assert facts["nix"]["trustedGroups"] == ["wheel"], facts
    machine.succeed(
        "mkdir -p /etc/nixos && cp -a /home/alice/cfg/. /etc/nixos/",
        "chown -R alice:wheel /etc/nixos && chown root /etc/nixos",
        "chmod -R g+rwX /etc/nixos && find /etc/nixos -type d -exec chmod 2775 {} +",
        # git refuses a work tree of root's to alice too, so she allows it herself, as admins do.
        "su - alice -c 'git config --global --add safe.directory /etc/nixos'",
        "su - alice -c 'cd /etc/nixos && git add -A && git commit -qm shared'",
    )
    owners = machine.succeed("stat -c %U:%G /etc/nixos/.git /etc/nixos/flake.lock /etc/nixos/flake.nix").split()
    assert set(owners) == {"alice:wheel"}, owners

    def shared(cmd):
        return machine.succeed(f"su - alice -c 'cd /etc/nixos && {cmd}'")

    # furnish looks there now: the module's two files, changed as if it said /etc/nixos.
    policy = json.loads(machine.succeed("cat /etc/furnish/policy.json"))
    policy["osSpecific"]["nixos"]["configuration"]["value"] = "/etc/nixos"
    facts["nix"]["configuration"] = "/etc/nixos"
    for path, value in [("/etc/furnish/policy.json", policy), ("/etc/furnish/nixos.json", facts)]:
        machine.succeed(f"rm {path} && echo {shlex.quote(json.dumps(value))} > {path}")
    machine.succeed("systemctl restart furnish")
    # Root can observe a match in the shared wheel-owned git checkout, and so can a service
    # request for alice. A writable untrusted directory must not furnish an external match.
    probe = shlex.quote(json.dumps({"name": "preconfigured", "options": {"system.stateVersion": "25.11"}}))
    def probe_shared(uid):
        return json.loads(machine.succeed(f"echo {probe} | FURNISH_SERVICE=1 FURNISH_REQUESTER_UID={uid} furnish nixos-resource test"))
    alice_uid = int(machine.succeed("id -u alice"))
    assert probe_shared(0)["managedBy"] == "nixos"
    assert probe_shared(alice_uid)["managedBy"] == "nixos"
    print(machine.succeed("su - alice -c 'furnish install htop' 2>&1"))
    assert list(apps()) == ["htop"] and system() == "${after}", system()
    assert shared("git log -1 --format=%s").strip() == "furnish: install htop"
    assert shared("git status --porcelain").strip() == ""
    # What root's commit added to the git folder stays the group's, and flake.lock alice's.
    added = machine.succeed("find /etc/nixos/.git -user root -type d ! -perm -g+w").strip()
    assert added == "", added
    assert machine.succeed("stat -c %U /etc/nixos/flake.lock").strip() == "alice"
    shared("echo '# alice' >> configuration.nix && git commit -qam 'a change of alice'")

    # The git folder belongs to bob, who isn't in wheel: root's git doesn't run in it.
    machine.succeed("chown bob /etc/nixos/.git")
    out = machine.fail("su - alice -c 'furnish uninstall htop' 2>&1")
    assert "/etc/nixos/.git belongs to bob" in out and "trustedGroups" in out, out
    assert list(apps()) == ["htop"] and system() == "${after}"
    machine.succeed("chown alice /etc/nixos/.git")

    # Another group that can change the folder: refused, and the message says how to trust it.
    machine.succeed("chgrp users /etc/nixos")
    assert not probe_shared(alice_uid)["_exist"]
    out = machine.fail("su - alice -c 'furnish uninstall htop' 2>&1")
    assert "the group users can change /etc/nixos" in out and "trustedGroups" in out, out
    assert list(apps()) == ["htop"] and system() == "${after}"
  '';
}
