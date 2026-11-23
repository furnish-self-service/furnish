{
  description = "furnish: self-service app catalog for an organization's computers";

  # Stable NixOS; furnish keeps to its Rust (rust-version in Cargo.toml). Admins can build it
  # against their own nixpkgs with `inputs.furnish.inputs.nixpkgs.follows = "nixpkgs"`.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  # Builds the Rust dependencies once for both packages (cargoArtifacts below).
  inputs.crane.url = "github:ipetkov/crane/v0.23.4";
  # Apps furnish installs into the NixOS configuration (Furnish/NixOS). Empty here; on a computer,
  # the configuration points it at furnish's local flake (docs/site/admins.md):
  #   inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";
  inputs.apps.url = "path:./nix/no-apps";
  # Apps furnish installs into a person's standalone Home Manager configuration. Empty here; the
  # configuration points it at furnish's local flake in that person's data folder:
  #   inputs.furnish.inputs.home-apps.url = "path:/home/alice/.local/share/furnish/home-apps";
  inputs.home-apps.url = "path:./nix/no-apps";

  # Builds of main, so admins download furnish instead of compiling it. Nix asks once whether to
  # trust these settings; alternatively add both to nix.settings (docs/site/admins.md).
  nixConfig = {
    extra-substituters = [ "https://furnish.cachix.org" ];
    extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
  };

  outputs = { self, nixpkgs, crane, apps, home-apps }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
      # GitHub flake locks identify a commit; a release tag or Actions run number is
      # generally unavailable inside the pure build. Keep the source identity visible.
      desktopRevision = self.rev or (if self ? dirtyRev then nixpkgs.lib.removeSuffix "-dirty" self.dirtyRev else "");
      desktopVersion = "${version}-alpha"
        + nixpkgs.lib.optionalString (desktopRevision != "") "+git.${builtins.substring 0 12 desktopRevision}"
        + nixpkgs.lib.optionalString (desktopRevision != "" && self ? dirtyRev) ".dirty";
      # Sources only: no build output, no node_modules.
      src = nixpkgs.lib.fileset.toSource {
        root = ./.;
        fileset = nixpkgs.lib.fileset.unions [
          ./Cargo.toml ./Cargo.lock ./src ./resources ./vendor ./packaging
          ./gui/package.json ./gui/package-lock.json ./gui/index.html ./gui/vite.config.js
          ./gui/src ./gui/src-tauri ./logo.svg
        ];
      };
      # DSC's own resources, which furnish includes (packaging/dsc.json): Microsoft's release, whose
      # programs are static, without samples and test tools, and File/Content built from the same source.
      dsc = builtins.fromJSON (builtins.readFile ./packaging/dsc.json);
      dscResources = pkgs:
        let
          release = pkgs.fetchurl { inherit (dsc.releases.${pkgs.stdenv.hostPlatform.system}) url sha256; };
          filecontent = pkgs.rustPlatform.buildRustPackage {
            pname = "dsc-filecontent";
            inherit (dsc) version;
            src = pkgs.fetchFromGitHub {
              owner = "PowerShell";
              repo = "DSC";
              rev = dsc.commit;
              hash = "sha256-BPAiKwKmE8WcLczAoXq6xjFWEpoWMqegz/9T9yYRBV8=";
            };
            # It points at Microsoft's internal crate mirror.
            postPatch = "rm -r .cargo";
            cargoHash = "sha256-xI2qyy3IA6nr6fAPaofDoIKCuin0Y8ipMyTL5Tx0zt8=";
            cargoBuildFlags = [ "-p" "filecontent" ];
            doCheck = false;
          };
        in pkgs.runCommand "dsc-resources-${dsc.version}" { } ''
          mkdir $out
          tar -xzf ${release} -C $out
          rm -f ${pkgs.lib.concatMapStringsSep " " (f: "$out/${f}") dsc.exclude}
          cp ${filecontent}/bin/filecontent ${filecontent.src}/resources/filecontent/filecontent.dsc.resource.json $out/
        '';
      # Shared by the dependency build and both packages: features, environment and build inputs
      # must be identical, or cargo rebuilds dependencies instead of reusing cargoArtifacts.
      rust = pkgs: rec {
        craneLib = crane.mkLib pkgs;
        commonArgs = {
          pname = "furnish-workspace";
          FURNISH_GIT_PATH = pkgs.lib.makeBinPath [ pkgs.git pkgs.git-lfs ];
          inherit version src;
          strictDeps = true;
          # One download of all crates (as buildRustPackage's cargoHash), in crane's layout.
          cargoVendorDir = let vendor = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            hash = "sha256-Si2do8IblJwZW+CsaHFg12yxKvEUB9/0i5v4qPLWGyM=";
          }; in pkgs.runCommand "furnish-vendor" { } ''
            mkdir $out
            sed "s|@vendor@|${vendor}|" ${vendor}/.cargo/config.toml > $out/config.toml
          '';
          # wrapGAppsHook3 changes PKG_CONFIG_PATH, which the GTK crates' build scripts track, so
          # every derivation has it; only furnish-gui wraps its program.
          nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook3 ];
          dontWrapGApps = true;
          buildInputs = with pkgs; [ glib-networking openssl webkitgtk_4_1 librsvg libayatana-appindicator ];
          # Let stdenv strip, as rustPlatform does.
          CARGO_PROFILE_RELEASE_STRIP = "false";
          doCheck = false; # tests: `nix flake check` (checks.tests) and CI
          doNotPostBuildInstallCargoBinaries = true; # installPhaseCommand below
        };
        cargoBuildCli = "cargo build --release --locked -p furnish";
        # tauri/custom-protocol: the app serves the built frontend (gui/dist), not Vite's dev server.
        cargoBuildGui = "cargo build --release --locked -p furnish-gui --features tauri/custom-protocol";
        # The dependency tree of both packages, compiled once; furnish and furnish-gui then only
        # compile their own crates. Both cargo commands run here because they resolve features
        # differently, which gives some dependencies two builds.
        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
          # crane stubs the workspace's own crates; the vendored grammar is a dependency of dsc-lib.
          dummySrc = craneLib.mkDummySrc {
            inherit src;
            extraDummyScript = "rm -rf $out/vendor && cp -r ${./vendor} $out/vendor";
          };
          buildPhaseCargoCommand = "${cargoBuildCli}\n${cargoBuildGui}";
        });
      };
      # Older CLI versions expect a list when the facts contain allowedOptions.
      optionFacts = lib: nix: lib.optionalAttrs (nix.allowedOptions != null) { inherit (nix) allowedOptions; };
      # Shared by both modules: what the installed apps (`apps` of an apps.json, as
      # src/nixos_apps.rs writes it) add, and whether the allowlist permits an option. Packages are
      # names in the configuration's nixpkgs.
      appsOf = lib: pkgs: allowedOptions: byName:
        let
          installed = lib.attrValues byName;
          # An entry of allowedOptions: an option's name, where `*` stands for one part.
          matches = pattern: name:
            let p = lib.splitString "." pattern; n = lib.splitString "." name; in
            lib.length p == lib.length n && lib.all lib.id (lib.zipListsWith (a: b: a == "*" || a == b) p n);
        in {
          appPackages = map (name: lib.attrByPath (lib.splitString "." name)
            (throw "furnish: nixpkgs has no package ${name}") pkgs) (lib.concatMap (a: a.packages or [ ]) installed);
          appSettings = lib.concatMap (a: lib.mapAttrsToList (name: value: { inherit name value; }) (a.options or { })) installed;
          allowed = name: allowedOptions == null || lib.any (p: matches p name) allowedOptions;
        };
      # The NixOS module, with the apps from furnish's local flake (the `apps` input).
      nixosModule = apps: { config, options, lib, pkgs, ... }:
        let
          cfg = config.services.furnish;
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
          # The apps furnish installed into the system (Furnish/NixOS), from its local flake.
          inherit (appsOf lib pkgs cfg.nix.allowedOptions ((apps.furnish or { }).apps or { })) appPackages appSettings allowed;
          # The local flake furnish keeps its apps in; also written by src/nixos_apps.rs.
          appsFlake = "{ outputs = _: { furnish = builtins.fromJSON (builtins.readFile ./apps.json); }; }";
          # Module first: what the configuration says wins over people's own setting.
          entry = value: { inherit value; locked = true; };
          nixos = lib.filterAttrs (_: v: v.value != null) {
            configuration = entry cfg.nix.configuration;
            flakeAttribute = entry cfg.nix.flakeAttribute;
          };
          # With the service on, furnish sends the preset collections' installs to it: the policy
          # gets a `service` section (src/service.rs `enabled`), with what `policy.service` sets.
          policy = lib.recursiveUpdate
            (lib.recursiveUpdate (lib.optionalAttrs cfg.service.enable { service = { }; }) (if cfg.policy == null then { } else cfg.policy))
            (lib.optionalAttrs (nixos != { }) { osSpecific.nixos = nixos; });
          # Whether sudo, sudo-rs or doas let every member of wheel run any command as root, as
          # the rules NixOS adds for wheel do: then wheel may change the configuration's folder
          # (`nix.trustedGroups`). sudo: a rule for the group with the command ALL, as root or ALL.
          # doas: a rule for the group without `cmd`, as root or anyone (`runAs` unset).
          sudoWheel = s: s.enable && lib.any (r:
            lib.elem "wheel" (r.groups or [ ])
            && lib.elem (lib.head (lib.splitString ":" (r.runAs or "ALL:ALL"))) [ "ALL" "root" ]
            && lib.any (c: (if builtins.isString c then c else c.command) == "ALL") (r.commands or [ ])) s.extraRules;
          doasWheel = d: d.enable && lib.any (r:
            lib.elem "wheel" (r.groups or [ ]) && lib.elem (r.runAs or null) [ null "root" ] && (r.cmd or null) == null) d.extraRules;
          wheelIsRoot = sudoWheel config.security.sudo
            || (options.security ? sudo-rs && sudoWheel config.security.sudo-rs)
            || doasWheel config.security.doas;
          # What the module knows when the system is built, for furnish (src/nixos.rs). The
          # configuration's folder only comes from the option: in a flake, `self` is a store copy.
          facts = {
            hostName = config.networking.hostName;
            configurationRevision = config.system.configurationRevision;
            homeManagerModule = options ? home-manager;
            # Per person with Home Manager: whether furnish's Home Manager module is on, and the
            # options apps may set there. From here, not from the person's home folder, which
            # they could change.
            homeManagerUsers = lib.optionalAttrs (options ? home-manager) (lib.mapAttrs (_: u: {
              inherit (u.services.furnish) enable;
            } // optionFacts lib u.services.furnish.nix) config.home-manager.users);
            moduleVersion = packages.furnish.version;
            nix = { inherit (cfg.nix) configuration flakeAttribute rebuild trustedGroups; } // optionFacts lib cfg.nix;
          };
        in {
          options.services.furnish = {
            enable = lib.mkEnableOption "furnish, the self-service app catalog";
            policy = lib.mkOption {
              type = lib.types.nullOr (pkgs.formats.json { }).type;
              default = null;
              description = "Admin policy (/etc/furnish/policy.json): settings, preset collections, options, service.";
              example = { collections.preset = [{ name = "acme-it"; url = "https://github.com/acme/it-apps.git"; }]; service.allowedGroup = "users"; };
            };
            service.enable = lib.mkEnableOption "the furnish service, which installs your organization's apps for users without administrator rights";
            # Where this computer's NixOS configuration is (see docs/site/admins.md). Written to
            # the policy as osSpecific.nixos.*; furnish searches only without it.
            nix = {
              configuration = lib.mkOption {
                # A string, not a path: a path literal would become a copy in the Nix store.
                type = lib.types.nullOr (lib.types.strMatching "/.*");
                default = null;
                description = "Folder of the NixOS configuration (with flake.nix or configuration.nix), as a string.";
                example = "/home/alex/cfg";
              };
              flakeAttribute = lib.mkOption {
                type = lib.types.nullOr lib.types.str;
                default = config.networking.hostName;
                defaultText = lib.literalExpression "config.networking.hostName";
                description = "Name in the flake's nixosConfigurations.";
                example = "alex-laptop";
              };
              allowedOptions = lib.mkOption {
                type = lib.types.nullOr (lib.types.listOf (lib.types.strMatching "[A-Za-z0-9_*-]+(\\.[A-Za-z0-9_*-]+)*"));
                default = null;
                description = "Optional restriction on the options apps may set. By default all options are permitted. A list permits the names it matches, with `*` matching one part of a name. An empty list permits only packages.";
                example = [ "programs.*.enable" "services.flatpak.enable" ];
              };
              trustedGroups = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = lib.optional wheelIsRoot "wheel";
                defaultText = lib.literalMD ''`[ "wheel" ]` when sudo, sudo-rs or doas let wheel run any command as root (the rules NixOS adds for wheel), else `[ ]`'';
                description = "Groups whose members may become root anyway. The furnish service rebuilds a configuration whose folder, or a folder above it, one of these groups can write to; it refuses other groups and folders everyone can write to.";
                example = [ "wheel" "nixos-admins" ];
              };
              rebuild = lib.mkOption {
                type = lib.types.enum [ "switch" "boot" ];
                default = "switch";
                description = "After installing an app into the configuration, the furnish service switches to the new system now, or makes it the one the computer starts next (boot).";
              };
            };
          };
          config = lib.mkIf cfg.enable (lib.mkMerge ([
          # With Home Manager's NixOS module: furnish's Home Manager module for every person, so
          # the configuration needs no line for it (services.furnish.enable there turns it on).
          (lib.optionalAttrs (options ? home-manager) {
            home-manager.sharedModules = [ (homeManagerModule { inherit apps home-apps; }) ];
          })
          {
            environment.systemPackages = [ packages.furnish packages.furnish-gui ] ++ appPackages;
            # Nix packages the furnish service installs for everyone (Furnish/Package as root).
            environment.profiles = lib.mkIf cfg.service.enable [ "/nix/var/nix/profiles/furnish" ];
            environment.etc."furnish/policy.json" = lib.mkIf (policy != { }) { text = builtins.toJSON policy; };
            environment.etc."furnish/nixos.json".text = builtins.toJSON facts;
            security.polkit.enable = true;
            # furnish's source and crane's stay in the store as long as this system does, so the
            # configuration still evaluates offline after a garbage collection.
            system.extraDependencies = [ self.outPath crane.outPath ];
            # The local flake the configuration's furnish/apps input points at; furnish fills it.
            systemd.tmpfiles.settings.furnish-apps = {
              "/var/lib/furnish/apps".d = { mode = "0755"; user = "root"; group = "root"; };
              "/var/lib/furnish/apps/flake.nix".f = { mode = "0644"; user = "root"; group = "root"; argument = appsFlake; };
              "/var/lib/furnish/apps/apps.json".f = { mode = "0644"; user = "root"; group = "root"; argument = ''{"version":1,"apps":{}}''; };
            };
            assertions = map (s: {
              assertion = allowed s.name;
              message = "furnish: an app sets ${s.name}, which isn't in services.furnish.nix.allowedOptions";
            }) appSettings;
            systemd.services.furnish = lib.mkIf cfg.service.enable {
              description = "furnish service: installs apps your organization provides";
              wantedBy = [ "multi-user.target" ];
              wants = [ "network-online.target" ];
              after = [ "network-online.target" ];
              # systemd-run: switching to a new system runs outside the service, which it may restart.
              path = with pkgs; [ bash coreutils git git-lfs nix config.systemd.package ];
              # It reads the policy once for its resources (FURNISH_CONFIG__*): restart on changes.
              restartTriggers = [ (builtins.toJSON policy) ];
              serviceConfig = {
                ExecStart = "${packages.furnish}/bin/furnish service";
                Restart = "on-failure";
                UMask = "0022";
                StateDirectory = "furnish";
              };
            };
          }] ++ map (s: lib.mkIf (allowed s.name) (lib.setAttrByPath (lib.splitString "." s.name) s.value)) appSettings));
        };
      # The Home Manager module, with the apps from furnish's local flakes: the person's own
      # (`home-apps`, standalone Home Manager) and the system's (`apps`, under homes.<user>, for
      # Home Manager as a NixOS module, where the furnish service installs them).
      # The key lets Nix import it once when a configuration also lists it itself.
      homeManagerModule = { apps, home-apps }: {
        key = "furnish#homeManagerModules.default";
        _file = "furnish/flake.nix#homeManagerModules.default";
        imports = [ (args@{ config, lib, pkgs, ... }:
        let
          cfg = config.services.furnish;
          user = config.home.username;
          own = (home-apps.furnish or { }).apps or { };
          homes = (apps.furnish or { }).homes or { };
          inherit (appsOf lib pkgs cfg.nix.allowedOptions (((homes.${user} or { }).apps or { }) // own))
            appPackages appSettings allowed;
          # Which options the module defines can't depend on config (home.username): every home's
          # settings, each only for its own user.
          settingsOf = mine: byName: map (s: s // { inherit mine; }) (appsOf lib pkgs [ ] byName).appSettings;
          definitions = settingsOf true own
            ++ lib.concatLists (lib.mapAttrsToList (name: h: settingsOf (name == user) (h.apps or { })) homes);
          # What the module knows when the generation is built, for furnish (src/nixos.rs).
          facts = {
            moduleVersion = version;
            nixosModule = args ? osConfig;
            nix = { inherit (cfg.nix) configuration flakeAttribute; } // optionFacts lib cfg.nix;
          };
        in {
          options.services.furnish = {
            enable = lib.mkEnableOption "the apps furnish installs into this Home Manager configuration";
            nix = {
              configuration = lib.mkOption {
                type = lib.types.nullOr (lib.types.strMatching "/.*");
                default = null;
                description = "Folder of the Home Manager configuration (with flake.nix), as a string.";
                example = "/home/alex/.config/home-manager";
              };
              flakeAttribute = lib.mkOption {
                type = lib.types.str;
                default = user;
                defaultText = lib.literalExpression "config.home.username";
                description = "Name in the flake's homeConfigurations (standalone Home Manager).";
                example = "alex@laptop";
              };
              allowedOptions = lib.mkOption {
                type = lib.types.nullOr (lib.types.listOf (lib.types.strMatching "[A-Za-z0-9_*-]+(\\.[A-Za-z0-9_*-]+)*"));
                default = null;
                description = "Optional restriction on the options apps may set. By default all options are permitted. A list permits the names it matches, with `*` matching one part of a name. An empty list permits only packages.";
                example = [ "programs.*.enable" ];
              };
            };
          };
          config = lib.mkIf cfg.enable (lib.mkMerge ([{
            home.packages = appPackages;
            xdg.configFile."furnish/home-manager.json".text = builtins.toJSON facts;
            # As system.extraDependencies in the NixOS module: the configuration still evaluates
            # offline after a garbage collection.
            home.extraDependencies = [ self.outPath crane.outPath ];
            assertions = map (s: {
              assertion = allowed s.name;
              message = "furnish: an app sets ${s.name}, which isn't in services.furnish.nix.allowedOptions";
            }) appSettings;
          }] ++ map (s: lib.mkIf (s.mine && allowed s.name) (lib.setAttrByPath (lib.splitString "." s.name) s.value)) definitions));
        }) ];
      };
    in {
      packages = forAll (pkgs: let r = rust pkgs; in rec {
        dsc-resources = dscResources pkgs;

        # The CLI (and the furnish service).
        furnish = r.craneLib.buildPackage (r.commonArgs // {
          pname = "furnish";
          inherit (r) cargoArtifacts;
          buildPhaseCargoCommand = r.cargoBuildCli;
          doInstallCargoArtifacts = false;
          installPhaseCommand = "install -Dm755 target/release/furnish -t $out/bin";
          # furnish finds DSC's resources in ../lib/furnish/dsc (engine.rs).
          postInstall = ''
            install -Dm644 packaging/linux/io.github.furnish.policy -t $out/share/polkit-1/actions
            mkdir -p $out/lib/furnish && ln -s ${dsc-resources} $out/lib/furnish/dsc
          '';
          meta = {
            description = "Self-service app catalog: install apps your organization curated in git";
            homepage = "https://github.com/furnish-self-service/furnish";
            license = pkgs.lib.licenses.mit;
            mainProgram = "furnish";
          };
        });

        # The desktop app: the frontend (npm), the binary (cargo), then Tauri's Debian bundle
        # layout (binary, desktop file, icons) into $out, as nixpkgs' cargo-tauri.hook does.
        furnish-gui = r.craneLib.buildPackage (r.commonArgs // {
          pname = "furnish-gui";
          version = desktopVersion;
          FURNISH_VERSION = desktopVersion;
          FURNISH_BUILD_REVISION = desktopRevision;
          SOURCE_DATE_EPOCH = if (self.lastModified or 0) > 1 then toString self.lastModified else "";
          inherit (r) cargoArtifacts;
          npmDeps = pkgs.fetchNpmDeps {
            name = "furnish-gui-npm-deps-${version}";
            src = "${src}/gui";
            hash = "sha256-OK0Pw41CdZTXH7csc323xVgP3FqvmY7sO3vqJqdmX6I=";
          };
          npmRoot = "gui";
          nativeBuildInputs = r.commonArgs.nativeBuildInputs
            ++ (with pkgs; [ cargo-tauri nodejs npmHooks.npmConfigHook ]);
          dontWrapGApps = false;
          preBuild = "(cd gui && npm run build)";
          buildPhaseCargoCommand = r.cargoBuildGui;
          doInstallCargoArtifacts = false;
          installPhaseCommand = ''
            (cd gui && cargo tauri bundle --bundles deb)
            mkdir -p $out
            mv target/release/bundle/deb/*/data/usr/* $out/
          '';
          postInstall = "mkdir -p $out/lib/furnish && ln -s ${dsc-resources} $out/lib/furnish/dsc";
          # The tray icon loads libayatana-appindicator at runtime. GStreamer: as cargo-tauri.hook.
          preFixup = ''
            gappsWrapperArgs+=(
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ]}
              --prefix WEBKIT_GST_ALLOWED_URI_PROTOCOLS : "asset"
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${pkgs.cargo-tauri.gst-plugin}/lib/gstreamer-1.0/"
            )
          '';
          meta = furnish.meta // { mainProgram = "furnish-gui"; };
        });

        default = furnish-gui;
      });

      # NixOS: services.furnish.enable = true installs the app and the CLI; `policy` writes
      # /etc/furnish/policy.json; `service.enable` runs the furnish service (users without
      # administrator rights). `furnish service install` doesn't work on NixOS: use this instead.
      nixosModules.default = nixosModule apps;

      # Home Manager: services.furnish.enable adds the apps furnish installed for this person.
      homeManagerModules.default = homeManagerModule { inherit apps home-apps; };

      # `nix flake check`: the packages and furnish's library tests (`nix build` runs no tests).
      checks = forAll (pkgs: let
        r = rust pkgs;
        offlineModule = import ./tests/nixos-apps/offline-module.nix self;
      in {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) furnish furnish-gui;
        tests = r.craneLib.cargoTest (r.commonArgs // {
          inherit (r) cargoArtifacts;
          doCheck = true;
          cargoTestExtraArgs = "-p furnish --lib";
        });
        # NixOS VM: packages from the system's configuration show as installed.
        nixos-declared = import ./tests/nixos-declared.nix { inherit pkgs; module = self.nixosModules.default; };
        # NixOS VM: apps from furnish's local flake, a nested input, end up in the system.
        nixos-apps = import ./tests/nixos-apps.nix { inherit pkgs nixpkgs; nixosModule = offlineModule; furnish = self; };
        # NixOS VM: installing such an app through the furnish service.
        nixos-install = import ./tests/nixos-install.nix {
          inherit pkgs nixpkgs;
          nixosModule = offlineModule;
          furnish = self;
          module = offlineModule apps;
        };
        # NixOS VM: apps from furnish's local flake in a person's data folder end up in their
        # standalone Home Manager configuration.
        home-manager = import ./tests/home-manager.nix {
          inherit pkgs nixpkgs homeManagerModule;
          furnish = self;
          module = self.nixosModules.default;
        };
        # Evaluation: Home Manager as a NixOS module gets each person's apps from the system's flake.
        home-manager-nixos = import ./tests/home-manager-nixos.nix { inherit pkgs nixpkgs nixosModule; furnish = self; };
        # NixOS VM: installing an app into Home Manager as a NixOS module, through the service.
        home-manager-nixos-install = import ./tests/home-manager-nixos-install.nix {
          inherit pkgs nixpkgs;
          nixosModule = offlineModule;
          furnish = self;
          module = offlineModule apps;
        };
      });
    };
}
