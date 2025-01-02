{
  description = "Rax";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.systems.url = "github:nix-systems/default";
  inputs.flake-utils = {
    url = "github:numtide/flake-utils";
    inputs.systems.follows = "systems";
  };
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";

  outputs = {
    nixpkgs,
    flake-utils,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
      in {
        devShells.default = pkgs.mkShell (with pkgs; {
          packages = [
            bashInteractive
            # Need to use gcc11 for CUDA compatibility purposes.
            gcc11 # Specify gcc11 instead of default gcc
            (rust-bin.stable.latest.default.override {
              extensions = ["rust-src"];
            })
          ];
          shellHook = ''
            export CC=${pkgs.gcc11}/bin/gcc
            export CXX=${pkgs.gcc11}/bin/g++
            export PATH=${pkgs.gcc11}/bin:$PATH
          '';
        });
      }
    );
}
