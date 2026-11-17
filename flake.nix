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

  # Builds of main, so admins download furnish instead of compiling it. Nix asks once whether to
  # trust these settings; alternatively add both to nix.settings (docs/site/admins.md).
  nixConfig = {
    extra-substituters = [ "https://furnish.cachix.org" ];
    extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
  };

  outputs = { self, nixpkgs, crane, apps }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
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
          inherit version src;
          strictDeps = true;
          # One download of all crates (as buildRustPackage's cargoHash), in crane's layout.
          cargoVendorDir = let vendor = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            hash = "sha256-Ga18jaD71TihDVnX5PwhzUrJj6mvUtGNRyxms3kz8MU=";
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
      # The NixOS module, with the apps from furnish's local flake (the `apps` input).
      nixosModule = apps: { config, options, lib, pkgs, ... }:
        let
          cfg = config.services.furnish;
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
          # The apps furnish installed (Furnish/NixOS), from its local flake: apps.json, as
          # src/nixos_apps.rs writes it. Packages are names in the system's nixpkgs.
          installed = lib.attrValues ((apps.furnish or { }).apps or { });
          appPackages = map (name: lib.attrByPath (lib.splitString "." name)
            (throw "furnish: nixpkgs has no package ${name}") pkgs) (lib.concatMap (a: a.packages or [ ]) installed);
          appSettings = lib.concatMap (a: lib.mapAttrsToList (name: value: { inherit name value; }) (a.options or { })) installed;
          # An entry of allowedOptions: an option's name, where `*` stands for one part.
          matches = pattern: name:
            let p = lib.splitString "." pattern; n = lib.splitString "." name; in
            lib.length p == lib.length n && lib.all lib.id (lib.zipListsWith (a: b: a == "*" || a == b) p n);
          allowed = name: lib.any (p: matches p name) cfg.nix.allowedOptions;
          # The local flake furnish keeps its apps in; also written by src/nixos_apps.rs.
          appsFlake = "{ outputs = _: { furnish = builtins.fromJSON (builtins.readFile ./apps.json); }; }";
          # Module first: what the configuration says wins over people's own setting.
          entry = value: { inherit value; locked = true; };
          nixos = lib.filterAttrs (_: v: v.value != null) {
            configuration = entry cfg.nix.configuration;
            flakeAttribute = entry cfg.nix.flakeAttribute;
          };
          policy = lib.recursiveUpdate (if cfg.policy == null then { } else cfg.policy)
            (lib.optionalAttrs (nixos != { }) { osSpecific.nixos = nixos; });
          # What the module knows when the system is built, for furnish (src/nixos.rs). The
          # configuration's folder only comes from the option: in a flake, `self` is a store copy.
          facts = {
            hostName = config.networking.hostName;
            configurationRevision = config.system.configurationRevision;
            homeManagerModule = options ? home-manager;
            moduleVersion = packages.furnish.version;
            nix = { inherit (cfg.nix) configuration flakeAttribute allowedOptions rebuild; };
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
                type = lib.types.listOf (lib.types.strMatching "[A-Za-z0-9_*-]+(\\.[A-Za-z0-9_*-]+)*");
                default = [ ];
                description = "NixOS options that apps (Furnish/NixOS) may set; `*` stands for one part of the name. Packages need no entry.";
                example = [ "programs.*.enable" "services.flatpak.enable" ];
              };
              rebuild = lib.mkOption {
                type = lib.types.enum [ "switch" "boot" ];
                default = "switch";
                description = "After installing an app into the configuration, the furnish service switches to the new system now, or makes it the one the computer starts next (boot).";
              };
            };
          };
          config = lib.mkIf cfg.enable (lib.mkMerge ([{
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

      # `nix flake check`: the packages and furnish's library tests (`nix build` runs no tests).
      checks = forAll (pkgs: let r = rust pkgs; in {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) furnish furnish-gui;
        tests = r.craneLib.cargoTest (r.commonArgs // {
          inherit (r) cargoArtifacts;
          doCheck = true;
          cargoTestExtraArgs = "-p furnish --lib";
        });
        # NixOS VM: packages from the system's configuration show as installed.
        nixos-declared = import ./tests/nixos-declared.nix { inherit pkgs; module = self.nixosModules.default; };
        # NixOS VM: apps from furnish's local flake, a nested input, end up in the system.
        nixos-apps = import ./tests/nixos-apps.nix { inherit pkgs nixpkgs nixosModule; furnish = self; };
        # NixOS VM: installing such an app through the furnish service.
        nixos-install = import ./tests/nixos-install.nix {
          inherit pkgs nixpkgs nixosModule;
          furnish = self;
          module = self.nixosModules.default;
        };
      });
    };
}
