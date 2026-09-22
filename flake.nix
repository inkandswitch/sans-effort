{
  description = "sans-effort — host-driven async coroutines whose every wait is a typed effect";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-26.05";
    # Only for `wasm-bindgen-cli`, which stable lags on and which must match
    # the crate version exactly.
    unstable-nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";

    command-utils.url = "git+https://tangled.org/expede.wtf/nix-command-utils";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    command-utils,
    flake-utils,
    nixpkgs,
    rust-overlay,
    unstable-nixpkgs,
  } @ inputs:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [
          (import rust-overlay)
        ];

        pkgs = import nixpkgs {
          inherit system overlays;
        };

        unstable = import unstable-nixpkgs {inherit system;};

        # Keep in lockstep with `rust-version` in Cargo.toml and rust-toolchain.toml.
        rustVersion = "1.91.0";

        rust-toolchain =
          (pkgs.rust-bin.stable.${rustVersion}.default.override {
            extensions = [
              "cargo"
              "clippy"
              "llvm-tools-preview"
              "rust-src"
              "rust-std"
              "rustfmt"
            ];

            targets = [
              "aarch64-apple-darwin"
              "x86_64-apple-darwin"

              "aarch64-unknown-linux-musl"
              "x86_64-unknown-linux-musl"

              "thumbv6m-none-eabi"
              "wasm32-unknown-unknown"
            ];
          }).overrideAttrs (old: {
            meta = (old.meta or {}) // {mainProgram = "cargo";};
          });

        format-pkgs = with pkgs; [
          alejandra
          taplo
        ];

        cargo-installs = with pkgs; [
          cargo-criterion
          cargo-deny
          cargo-hack
          cargo-nextest
          cargo-semver-checks
          cargo-watch
          typos
        ];

        # The demo's foreign hosts. `wasm-bindgen-cli` must match the `=` pin on
        # the `wasm-bindgen` crate in Cargo.toml; bump both together.
        demo-pkgs = [
          pkgs.jdk25 # java.lang.foreign is final from 22
          pkgs.nodejs
          pkgs.python3
          unstable.wasm-bindgen-cli
        ];

        # xdg-utils ships several binaries and sets no mainProgram; name the one
        # `rust.bench` opens reports with, so `lib.getExe` doesn't have to guess.
        xdg-open = pkgs.xdg-utils.overrideAttrs (old: {
          meta = (old.meta or {}) // {mainProgram = "xdg-open";};
        });

        # Built-in command modules from nix-command-utils
        rust = command-utils.rust.${system};
        cmd = command-utils.cmd.${system};
        asModule = command-utils.asModule.${system};

        # Project-specific commands
        projectCommands = import ./nix/commands.nix {
          inherit pkgs system cmd;
          wasm-bindgen-cli = unstable.wasm-bindgen-cli;
        };

        command_menu = command-utils.commands.${system} [
          (rust.build {cargo = rust-toolchain;})
          (rust.test {
            cargo = rust-toolchain;
            cargo-watch = pkgs.cargo-watch;
          })
          (rust.lint {cargo = rust-toolchain;})
          (rust.fmt {cargo = rust-toolchain;})
          (rust.doc {cargo = rust-toolchain;})
          (rust.bench {
            cargo = rust-toolchain;
            cargo-criterion = pkgs.cargo-criterion;
            xdg-open = xdg-open;
          })
          (rust.watch {cargo-watch = pkgs.cargo-watch;})
          (rust.audit {cargo-audit = pkgs.cargo-audit;})
          (rust.semver {cargo-semver-checks = pkgs.cargo-semver-checks;})
          (rust.ci {cargo = rust-toolchain;})

          (asModule projectCommands)
        ];

        # Minimal CI toolchain — no llvm-tools, no rust-src, fewer targets
        ci-rust-toolchain =
          (pkgs.rust-bin.stable.${rustVersion}.default.override {
            extensions = [
              "cargo"
              "clippy"
              "rustfmt"
            ];

            targets = [
              "thumbv6m-none-eabi"
              "wasm32-unknown-unknown"
            ];
          }).overrideAttrs (old: {
            meta = (old.meta or {}) // {mainProgram = "cargo";};
          });

        # Stub so tools that probe for rustup (e.g. cargo-semver-checks)
        # silently succeed instead of failing with "No such file or directory".
        rustup-shim = pkgs.writeShellScriptBin "rustup" ''
          if [ "$1" = "--version" ]; then
            echo "rustup-nix-shim"
          elif [ "$1" = "toolchain" ]; then
            exit 0
          elif [ "$1" = "which" ]; then
            command -v "$2" 2>/dev/null || echo "$2"
          else
            exit 0
          fi
        '';

        ci-cargo-installs = with pkgs; [
          cargo-deny
          cargo-hack
          cargo-semver-checks
          typos
        ];

        ci-demo-pkgs = demo-pkgs;
      in rec {
        devShells.default = pkgs.mkShell {
          name = "sans-effort-shell";

          # `command_menu` is already a flat list (menu + command scripts).
          nativeBuildInputs =
            command_menu
            ++ [
              rust-toolchain

              pkgs.rust-analyzer
            ]
            ++ format-pkgs
            ++ cargo-installs
            ++ demo-pkgs
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.clang
              pkgs.llvmPackages.libclang
            ];

          shellHook = ''
            unset SOURCE_DATE_EPOCH
            export WORKSPACE_ROOT="$(pwd)"
            menu
          '';
        };

        devShells.ci = pkgs.mkShell {
          name = "sans-effort-ci";

          nativeBuildInputs =
            [
              ci-rust-toolchain
              rustup-shim
            ]
            ++ ci-cargo-installs
            ++ ci-demo-pkgs;
        };

        formatter = pkgs.alejandra;
      }
    );
}
