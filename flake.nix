{
  description = "Orby: an OrbStack-backed NixOS remote builder for nix-darwin";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "aarch64-darwin";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      darwinModules.default = import ./module { inherit self; };
      darwinModules.orby = self.darwinModules.default;

      packages.${system} = {
        default = self.packages.${system}.orby;
        orby = pkgs.callPackage ./pkgs/orby { };
      };

      checks.${system}.module-eval =
        assert import ./tests/module-eval.nix { inherit self nixpkgs; };
        pkgs.runCommand "orby-module-eval" { } "touch $out";

      devShells = nixpkgs.lib.genAttrs [ "aarch64-darwin" "aarch64-linux" "x86_64-linux" ] (system: {
        default =
          let
            pkgs = nixpkgs.legacyPackages.${system};
          in
          pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              nixfmt
            ];
            shellHook = ''
              export CARGO_HOME="$PWD/.cache/cargo"
              export CARGO_TARGET_DIR="$PWD/.cache/target"
              export RUSTUP_HOME="$PWD/.cache/rustup"
              mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR" "$RUSTUP_HOME"
            '';
          };
      });
      formatter.${system} = pkgs.nixfmt-tree.override {
        settings.formatter.nixfmt.options = [ "--strict" ];
      };
    };
}
