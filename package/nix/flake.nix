{
  description = "wprs (wprsc client + wprsd server)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.wprsc = pkgs.rustPlatform.buildRustPackage {
          pname = "wprs";
          version = "@VERSION@";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          # Client-only by default; server requires extra system deps and is Linux-specific.
          buildFeatures = [ "winit-pixels-client" ];
          buildNoDefaultFeatures = false;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.wayland
            pkgs.libxkbcommon
          ];

          cargoBuildFlags = [ "--bin" "wprsc" "--profile=release-lto" ];
          installPhase = ''
            mkdir -p $out/bin
            cp target/release-lto/wprsc $out/bin/
          '';
        };

        packages.default = self.packages.${system}.wprsc;
      });
}
