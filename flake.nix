{
  description = "Crimson Desert mods - desert-core + the Desert Looter / Desert Gatherer ASI plugins (Rust, cross-compiled to Windows x64)";

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
            # Ghidra itself is NOT here any more - it runs on the Windows side
            # and is driven through the GhidraMCP server; see CLAUDE.md.
            pkgs.python3
            pkgs.binutils
            pkgs.file

            # dmm-pack/rebase.py rewrites the DMM offset patches; jq is for
            # eyeballing those JSONs without loading a 230 KB file into an editor.
            pkgs.jq
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
            echo "Crimson Desert mods workspace ready (run cargo from the repo root)."
            echo "  build:   cargo build --release"
            echo "  test:    cargo test --target x86_64-unknown-linux-gnu"
            echo "  output:  target/x86_64-pc-windows-gnu/release/desert_looter.dll   -> bin64/DesertLooter.asi"
            echo "           target/x86_64-pc-windows-gnu/release/desert_gatherer.dll -> bin64/DesertGatherer.asi"
          '';
        };
      });
}
