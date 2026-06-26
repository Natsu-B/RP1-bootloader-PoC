{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          targets = [ "aarch64-unknown-none-softfloat" "aarch64-unknown-uefi" ];
          extensions = [ "rust-src" "llvm-tools-preview" ];
        };
        cargoNightlyCompat = pkgs.writeShellScriptBin "cargo" ''
          if [ "''${1:-}" = "+nightly" ]; then
            shift
          fi
          exec ${rustToolchain}/bin/cargo "$@"
        '';
        buildBootloader = pkgs.writeShellScriptBin "build-bootloader" ''
          exec cargo xbuild "$@"
        '';
        buildBootloaderTftp = pkgs.writeShellScriptBin "build-bootloader-tftp" ''
          exec cargo xbuild --features tftp-boot "$@"
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            cargoNightlyCompat
            buildBootloader
            buildBootloaderTftp
            rustToolchain
            pkgs.binutils
            pkgs.xxd
            pkgs.dtc
            pkgs.cargo-binutils
            pkgs.dnsmasq
            pkgs.tcpdump
          ];
        };
      }
    );
}
