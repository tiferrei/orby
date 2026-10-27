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
        default = self.packages.${system}.orby-runner;
        orby-runner = pkgs.callPackage ./pkgs/orby-runner { };
      };

      checks.${system}.module-eval =
        assert import ./tests/module-eval.nix { inherit self nixpkgs; };
        pkgs.runCommand "orby-module-eval" { } "touch $out";

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          go
          gopls
          gofumpt
          jq
          nixfmt
        ];
        shellHook = ''
          export GOPATH="$PWD/.cache/go"
          export GOMODCACHE="$GOPATH/pkg/mod"
          export GOCACHE="$PWD/.cache/go-build"
          export GOTOOLCHAIN=local
          mkdir -p "$GOMODCACHE" "$GOCACHE"
        '';
      };
      formatter.${system} = pkgs.nixfmt-tree.override {
        settings.formatter.nixfmt.options = [ "--strict" ];
      };
    };
}
