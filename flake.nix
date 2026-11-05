{
  description = "NoFlex - a self-hosted media server in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    crane,
  }:
    flake-utils.lib.eachSystem ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"] (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        # Use the toolchain pinned in backend/rust-toolchain.toml so the Nix build
        # matches the cargo build exactly.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./backend/rust-toolchain.toml;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Separate nightly rustfmt for the unstable `imports_granularity` and
        # `group_imports` options that enforce the project's nested-grouping
        # import style. Invoked via `cargo +nightly fmt`.
        rustfmtNightly = pkgs.rust-bin.nightly.latest.rustfmt;

        # Runtime dependencies we need at both build and run time.
        runtimeDeps = with pkgs; [
          ffmpeg-full
          onnxruntime
          sqlite
        ];

        web = pkgs.buildNpmPackage {
          pname = "noflex-web";
          version = "0.0.1";
          # Houdini validates every document against the server's committed
          # SDL snapshot, so it travels with the web sources at the same
          # relative path. Local installs and build output stay out.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              (pkgs.lib.fileset.difference ./web (pkgs.lib.fileset.unions (map pkgs.lib.fileset.maybeMissing [
                ./web/node_modules
                ./web/build
                ./web/.svelte-kit
                ./web/.houdini
              ])))
              ./backend/crates/noflex-graphql/tests/snapshots/schema.graphql
            ];
          };
          sourceRoot = "source/web";
          # Houdini's compiler ships as prebuilt Go binaries in npm packages.
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [pkgs.autoPatchelfHook];
          preBuild = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            autoPatchelf node_modules/houdini-core-linux-* node_modules/houdini-svelte-linux-*
          '';
          npmDepsHash = "sha256-4HBuNu2qNe0VS6pi2vbQc/QmpeIBHQi0RbfQMKXkzvg=";
          npmBuildScript = "build";
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp -r build/. $out/
            runHook postInstall
          '';
        };

        # The Cargo workspace lives in backend/; the flake root is the
        # monorepo-wide dev shell, not a crate source.
        # cleanCargoSource drops non-Cargo files, but the build needs
        # .sql migrations (include_str! in the storage backends),
        # .wit worlds (wasmtime bindgen! in noflex-plugin-host), and
        # the .graphql SDL snapshot the schema-drift test diffs against.
        src = pkgs.lib.cleanSourceWith {
          src = pkgs.lib.cleanSource ./backend;
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*\\.(sql|wit|graphql)$" (toString path) != null);
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          # Clippy and the test suite run through cargo (dev profile) in
          # CI; the Nix build only produces the release package, so no
          # ThinLTO compile is spent on test targets.
          doCheck = false;
          buildInputs = runtimeDeps;
          nativeBuildInputs = with pkgs; [pkg-config];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        webArgs = commonArgs // {NOFLEX_WEB_DIR = web;};

        noflex = craneLib.buildPackage (webArgs
          // {
            inherit cargoArtifacts;
            # Wrap the binary so ffmpeg is on PATH at runtime.
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [pkgs.makeWrapper];
            postInstall = ''
              wrapProgram $out/bin/noflex-server \
                --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps} \
                --set-default ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime${pkgs.stdenv.hostPlatform.extensions.sharedLibrary}
            '';
            meta = with pkgs.lib; {
              description = "A self-hosted media server in Rust";
              license = licenses.agpl3Only;
              mainProgram = "noflex-server";
              platforms = platforms.unix;
            };
          });
      in {
        packages.default = noflex;
        packages.noflex = noflex;
        packages.web = web;

        apps.default = flake-utils.lib.mkApp {
          drv = noflex;
          name = "noflex-server";
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [noflex];
          packages = with pkgs; [
            rustToolchain
            rustfmtNightly
            nodejs
            cargo-watch
            cargo-nextest
            cargo-deny
            cargo-public-api
            cargo-component
            # Release bundles link against an old glibc; see docs/PACKAGING.md.
            cargo-zigbuild
            zig
            rust-analyzer
            sccache
            sqlite-interactive
            nixpkgs-fmt
            # WASM plugin toolchain.
            wasm-tools
            wit-bindgen
          ];

          # Point `cargo fmt` at nightly rustfmt so it honors the
          # `imports_granularity = "Crate"` and `group_imports = ...` options
          # in `rustfmt.toml`. Stable rustfmt warns and silently skips them.
          RUSTFMT = "${rustfmtNightly}/bin/rustfmt";
          # Test threads default to 2 MiB, which the merged GraphQL root's
          # debug-build futures outgrow; the server sets its own workers to match.
          RUST_MIN_STACK = "8388608";
          # ort loads ONNX Runtime at runtime rather than linking it.
          ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime${pkgs.stdenv.hostPlatform.extensions.sharedLibrary}";

          shellHook = ''
            # Create cache directories
            export PROJECT_ROOT="$(git rev-parse --git-common-dir | xargs dirname | xargs realpath)"
            mkdir -p "$PROJECT_ROOT/.cache"

            # Set up sccache for Rust
            export RUSTC_WRAPPER=sccache
            # Default to local cache in the root of the repo
            export SCCACHE_DIR="''${SCCACHE_DIR:-$PROJECT_ROOT/.cache/sccache}"
            # Set local cache size to be bigger than default
            export SCCACHE_CACHE_SIZE="''${SCCACHE_CACHE_SIZE:-8G}"
            # Set a separate port so NoFlex gets a unique port
            export SCCACHE_SERVER_PORT=42225

            echo "🎬 NoFlex development environment loaded!"
            echo ""
            echo "Language runtimes:"
            echo "  - 🦀 Rust:      $(rustc --version 2>/dev/null || echo 'not found')"
            echo "  - ✨ rustfmt:   $(rustfmt --version 2>/dev/null || echo 'not found')"
            echo ""
            echo "Media toolchain:"
            echo "  - 🎞️ ffmpeg:    $(ffmpeg -version 2>/dev/null | head -1 | awk '{print $3}' || echo 'not found')"
            echo "  - 🗃️ sqlite:    $(sqlite3 --version 2>/dev/null | awk '{print $1}' || echo 'not found')"
            echo ""
            echo "Cargo tools:"
            echo "  - 🧪 nextest:   $(cargo nextest --version 2>/dev/null | head -1 | awk '{print $2}' || echo 'not found')"
            echo "  - 👁️ watch:     $(cargo watch --version 2>/dev/null | awk '{print $2}' || echo 'not found')"
            echo "  - 🛡️ deny:      $(cargo deny --version 2>/dev/null | awk '{print $2}' || echo 'not found')"
            echo ""
            echo "Caching:"
            echo "  - ⚡️ Using sccache with $SCCACHE_CACHE_SIZE limit at $SCCACHE_DIR"
            echo "    * SCCACHE_SERVER_PORT: $SCCACHE_SERVER_PORT"
            if [ -n "$SCCACHE_GHA_ENABLED" ]; then
              echo "    * SCCACHE_GHA_ENABLED: $SCCACHE_GHA_ENABLED"
            fi
            echo ""
            echo "WASM toolchain (for plugins):"
            echo "  - 🕸️ wasm-tools:      $(wasm-tools --version 2>/dev/null | awk '{print $2}' || echo 'not found')"
            echo "  - 🔗 wit-bindgen:     $(wit-bindgen --version 2>/dev/null | awk '{print $2}' || echo 'not found')"
            echo "  - 📦 cargo-component: $(cargo component --version 2>/dev/null | awk '{print $2}' || echo 'not found')"
            echo ""
            echo "Quickstart (Cargo workspace lives in backend/):"
            echo "  - 🏗️ build:    cd backend && cargo build"
            echo "  - 🚀 run:      cd backend && cargo run -p noflex-server"
            echo "  - ✅ test:     cd backend && cargo nextest run --workspace"
            echo "  - 🎨 fmt:      cd backend && cargo fmt --all     (uses nightly rustfmt via \$RUSTFMT)"
            echo "  - 🔍 lint:     cd backend && cargo clippy --workspace --all-targets -- -D warnings"
            echo ""
          '';
        };

        checks = {
          inherit noflex;
          fmt = craneLib.cargoFmt {inherit src;};
        };
      }
    )
    // {
      # NixOS module: services.noflex.{enable, dataDir, ...}
      nixosModules.default = import ./nix/module.nix;
      nixosModules.noflex = self.nixosModules.default;
    };
}
