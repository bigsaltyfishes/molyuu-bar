{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils}:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          pkgs = import nixpkgs {
            inherit system;
          };
        in
        with pkgs;
        {
          devShells.default = mkShell {
            LIBCLANG_PATH="${llvmPackages.libclang.lib}/lib";
            buildInputs = [
              dart-sass
              pkg-config
              gtk4
              gtk4-layer-shell
              libadwaita
              librsvg
              clang
              niri
              foot
            ];
          };
        }
      );
}
