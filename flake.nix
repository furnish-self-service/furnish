{
  description = "furnish: self-service app catalog for an organization's computers";

  # Stable NixOS; furnish keeps to its Rust (rust-version in Cargo.toml). Admins can build it
  # against their own nixpkgs with `inputs.furnish.inputs.nixpkgs.follows = "nixpkgs"`.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  # Builds the Rust dependencies once for both packages (cargoArtifacts below).
  inputs.crane.url = "github:ipetkov/crane/v0.23.4";

  # Builds of main, so admins download furnish instead of compiling it. Nix asks once whether to
  # trust these settings; alternatively add both to nix.settings (docs/site/admins.md).
  nixConfig = {
    extra-substituters = [ "https://furnish.cachix.org" ];
    extra-trusted-public-keys = [ "furnish.cachix.org-1:2kYBYAHpsUEykPAKjJ9a6IlwDPwjnq565I6cPtASA6k=" ];
  };

  outputs = { self, nixpkgs, crane }:
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
            hash = "sha256-M4ek8APyeUnlFjePWYAd3/xNHmBlOTG/vI0qkxYMVC4=";
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
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.furnish;
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
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
          };
          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ packages.furnish packages.furnish-gui ];
            # Nix packages the furnish service installs for everyone (Furnish/Package as root).
            environment.profiles = lib.mkIf cfg.service.enable [ "/nix/var/nix/profiles/furnish" ];
            environment.etc."furnish/policy.json" = lib.mkIf (cfg.policy != null) { text = builtins.toJSON cfg.policy; };
            security.polkit.enable = true;
            systemd.services.furnish = lib.mkIf cfg.service.enable {
              description = "furnish service: installs apps your organization provides";
              wantedBy = [ "multi-user.target" ];
              wants = [ "network-online.target" ];
              after = [ "network-online.target" ];
              path = with pkgs; [ bash coreutils git git-lfs nix ];
              serviceConfig = {
                ExecStart = "${packages.furnish}/bin/furnish service";
                Restart = "on-failure";
                UMask = "0022";
                StateDirectory = "furnish";
              };
            };
          };
        };

      # `nix flake check`: the packages and furnish's library tests (`nix build` runs no tests).
      checks = forAll (pkgs: let r = rust pkgs; in {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) furnish furnish-gui;
        tests = r.craneLib.cargoTest (r.commonArgs // {
          inherit (r) cargoArtifacts;
          doCheck = true;
          cargoTestExtraArgs = "-p furnish --lib";
        });
      });
    };
}
