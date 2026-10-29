{
  description = "Lonk: a pure, typed, lazy language for embedding";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "lonk";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./LICENSE ./crates ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "lonk-cli" ];
          cargoTestFlags = [ "--workspace" ];
          postInstall = ''
            mkdir -p "$out/share/doc/lonk"
            cp LICENSE "$out/share/doc/lonk/"
          '';
          meta = {
            description = "Pure, typed, lazy language with snapshot values";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "lonk";
            platforms = pkgs.lib.platforms.unix;
          };
        };
      });
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [ rustfmt clippy rust-analyzer tree-sitter neovim ];
          env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
      checks = forAllSystems (pkgs: {
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });
    };
}
