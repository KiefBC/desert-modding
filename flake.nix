{
  description = "Desert Looter - Crimson Desert gathering auto-loot ASI (Rust, cross-compiled to Windows x64)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # Rust toolchain with the Windows GNU target added.
        toolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "x86_64-pc-windows-gnu" ];
        };

        # mingw-w64 cross toolchain provides the linker: x86_64-w64-mingw32-gcc
        mingw = pkgs.pkgsCross.mingwW64;
        # Rust's windows-gnu target links -l:libpthread.a (winpthreads); the
        # Nix mingw cc does not put it on the search path by itself.
        pthreads = mingw.windows.pthreads;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            toolchain
            mingw.stdenv.cc

            # Binary-analysis tooling for reverse-engineering the game and the
            # reference mod: tools/sigscan.py, strings, objdump, file.
            pkgs.python3
            pkgs.binutils
            pkgs.file

            # Disassembler/decompiler for reading CrimsonDesert.exe and the
            # reference mod. Prebuilt upstream release (ghidra-bin); the
            # from-source `ghidra` attribute is a very long build. GUI needs
            # WSLg; launchers are `ghidra` and `ghidra-analyzeHeadless`.
            pkgs.ghidra-bin
          ];

          # Cargo uses this linker for the gnu target. Scoped to this shell, so
          # nothing is hardcoded to a machine-specific path.
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER =
            "${mingw.stdenv.cc}/bin/x86_64-w64-mingw32-gcc";

          # NOTE: this env var REPLACES target.<triple>.rustflags from
          # .cargo/config.toml, so every flag for the gnu target lives here.
          #   +crt-static  -> no mingw runtime DLL dependencies
          #   -L native    -> where libpthread.a lives
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS =
            "-C target-feature=+crt-static -L native=${pthreads}/lib";

          shellHook = ''
            echo "Desert Looter dev shell ready."
            echo "  build:   cargo build --release"
            echo "  output:  target/x86_64-pc-windows-gnu/release/desert_looter.dll"
            echo "  install: copy that file to the game's bin64 as DesertLooter.asi"
            echo "  ghidra:  ghidra (GUI via WSLg) or ghidra-analyzeHeadless (batch)"
          '';
        };
      });
}
