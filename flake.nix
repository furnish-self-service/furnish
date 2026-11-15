{
  description = "furnish: self-service app catalog for an organization's computers";

  # Stable NixOS; furnish keeps to its Rust (rust-version in Cargo.toml). Admins can build it
  # against their own nixpkgs with `inputs.furnish.inputs.nixpkgs.follows = "nixpkgs"`.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

  outputs = { self, nixpkgs }:
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
      cargoLock = {
        lockFile = ./Cargo.lock;
        outputHashes."dsc-lib-3.2.0" = "sha256-BPAiKwKmE8WcLczAoXq6xjFWEpoWMqegz/9T9yYRBV8=";
      };
    in {
      packages = forAll (pkgs: rec {
        dsc-resources = dscResources pkgs;

        # The CLI (and the furnish service).
        furnish = pkgs.rustPlatform.buildRustPackage {
          pname = "furnish";
          inherit version src cargoLock;
          cargoBuildFlags = [ "-p" "furnish" ];
          cargoTestFlags = [ "-p" "furnish" "--lib" ];
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
        };

        # The desktop app.
        furnish-gui = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
          pname = "furnish-gui";
          inherit version src cargoLock;
          npmDeps = pkgs.fetchNpmDeps {
            name = "furnish-gui-npm-deps-${version}";
            src = "${finalAttrs.src}/gui";
            hash = "sha256-OK0Pw41CdZTXH7csc323xVgP3FqvmY7sO3vqJqdmX6I=";
          };
          npmRoot = "gui";
          buildAndTestSubdir = "gui";
          nativeBuildInputs = with pkgs; [ cargo-tauri.hook nodejs npmHooks.npmConfigHook pkg-config wrapGAppsHook3 ];
          buildInputs = with pkgs; [ glib-networking openssl webkitgtk_4_1 librsvg ];
          doCheck = false; # the library's tests run in `furnish`
          postInstall = "mkdir -p $out/lib/furnish && ln -s ${dsc-resources} $out/lib/furnish/dsc";
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

      checks = forAll (pkgs: {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) furnish furnish-gui;
      });
    };
}
