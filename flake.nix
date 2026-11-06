{
  description = "NoFlex - a self-hosted media server in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Flutter and the Android SDK follow their own pin so bumping them never
    # rebuilds the server, and so the unfree Android license stays out of the
    # package set the server is built from.
    nixpkgs-flutter.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
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
    nixpkgs-flutter,
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

        # ---------------------------------------------------------------
        # Dev shells, composed from these groups
        # ---------------------------------------------------------------
        #
        # `default` is everything. People who never touch one side of the repo
        # opt out with `no-flutter` (skips the Android SDK and its gigabytes) or
        # `flutter-only`. See CONTRIBUTING.md for pinning a choice under direnv.

        # `just` and what its recipes call; see tasks/ and scripts/tasks/.
        taskPkgs = with pkgs; [just curl jq unzip shellcheck shfmt];

        rustPkgs = with pkgs; [
          rustToolchain
          rustfmtNightly
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

        webPkgs = [pkgs.nodejs];

        rustEnv = {
          # Point `cargo fmt` at nightly rustfmt so it honors the
          # `imports_granularity = "Crate"` and `group_imports = ...` options
          # in `rustfmt.toml`. Stable rustfmt warns and silently skips them.
          RUSTFMT = "${rustfmtNightly}/bin/rustfmt";
          # Test threads default to 2 MiB, which the merged GraphQL root's
          # debug-build futures outgrow; the server sets its own workers to match.
          RUST_MIN_STACK = "8388608";
          # ort loads ONNX Runtime at runtime rather than linking it.
          ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime${pkgs.stdenv.hostPlatform.extensions.sharedLibrary}";
        };

        rustShellHook = ''
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
          echo "Quickstart:"
          echo "  - 📋 commands: just"
          echo "  - 🚀 run:      just server run"
          echo "  - 🎬 demo:     just demo up --servers 2"
          echo "  - ✅ check:    just check crate <crate>"
          echo ""
        '';

        # ---- Flutter + Android -------------------------------------------
        pkgsFlutter = import nixpkgs-flutter {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };
        flutter = pkgsFlutter.flutter347;
        jdk = pkgsFlutter.jdk17;

        # Keep in step with flutter/android: AGP can't install anything into the
        # read-only store, so every compileSdk and the NDK a plugin asks for
        # must be composed here.
        androidBuildTools = "36.0.0";
        # The system images and emulator are what fill a CI runner's disk, and
        # only running the app on a virtual device needs them.
        composeAndroidSdk = {emulator}:
          ((pkgsFlutter.androidenv.override {licenseAccepted = true;}).composeAndroidPackages {
            buildToolsVersions = [androidBuildTools "35.0.0"];
            cmakeVersions = ["3.22.1"];
            platformVersions = ["36" "35" "34" "33"];
            abiVersions = ["x86_64" "arm64-v8a"];
            # Google publishes no emulator for arm64 Linux.
            includeEmulator = emulator && !(pkgsFlutter.stdenv.hostPlatform.isLinux && pkgsFlutter.stdenv.hostPlatform.isAarch64);
            includeSystemImages = emulator;
            systemImageTypes = ["google_apis_playstore"];
            includeNDK = true;
            ndkVersions = ["28.2.13676358"];
          }).androidsdk;
        androidSdk = composeAndroidSdk {emulator = true;};
        androidBuildSdk = composeAndroidSdk {emulator = false;};

        # mpv.pc lists every optional library mpv was built with, and pkg-config
        # insists on them all just to compile against its client header.
        mpvClient = pkgsFlutter.runCommand "mpv-client-pc" {} ''
          mkdir -p $out/lib/pkgconfig
          sed '/^Requires.private/d' ${pkgsFlutter.mpv-unwrapped.dev}/lib/pkgconfig/mpv.pc > $out/lib/pkgconfig/mpv.pc
        '';

        # What the Linux desktop build needs beyond Flutter's own GTK: the
        # player's libmpv and the keyring behind secure storage.
        flutterLinuxLibs = with pkgsFlutter;
          lib.optionals stdenv.hostPlatform.isLinux [
            mpvClient
            libepoxy
            libglvnd
            libsecret
            # The video plugin includes GDK's Wayland header, which includes this.
            wayland
          ];

        flutterBasePkgs = with pkgsFlutter;
          [
            flutter
            git
            curl
            unzip
            pkg-config
          ]
          # path_provider asks this for the documents folder on Linux.
          ++ lib.optional stdenv.hostPlatform.isLinux xdg-user-dirs;

        flutterEnv = {
          # The wrapper only sets this when unset, and a stale value makes it
          # write a broken `flutter.sdk` into android/local.properties.
          FLUTTER_ROOT = "${flutter}";
        };

        androidEnvFor = sdk: let
          root = "${sdk}/libexec/android-sdk";
        in {
          # ANDROID_SDK_ROOT is deprecated, but older tools still consult it.
          ANDROID_HOME = root;
          ANDROID_SDK_ROOT = root;
          JAVA_HOME = "${jdk.home}";
          # Gradle's bundled aapt2 is a glibc ELF that won't run on NixOS.
          GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${root}/build-tools/${androidBuildTools}/aapt2";
        };

        flutterPkgs = flutterBasePkgs ++ [jdk androidSdk];
        flutterAndroidEnv = flutterEnv // androidEnvFor androidSdk;

        flutterShellHook = ''
          export PROJECT_ROOT="$(git rev-parse --git-common-dir | xargs dirname | xargs realpath)"
          # Keep AVDs with the repo so they don't collide on a shared ~/.android/avd.
          export ANDROID_AVD_HOME="$PROJECT_ROOT/.cache/android/avd"
          mkdir -p "$ANDROID_AVD_HOME"

          ${pkgsFlutter.lib.optionalString pkgsFlutter.stdenv.hostPlatform.isLinux ''
            # media_kit opens libmpv by name at runtime rather than linking it.
            export LD_LIBRARY_PATH="${pkgsFlutter.lib.makeLibraryPath [pkgsFlutter.mpv-unwrapped]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          ''}
          echo "🦋 Flutter ${flutter.version}''${ANDROID_HOME:+, Java ${jdk.version}, Android SDK at $ANDROID_HOME}"
          echo "   Test devices: see flutter/AGENTS.md"
          echo ""
        '';
      in {
        packages.default = noflex;
        packages.noflex = noflex;
        packages.web = web;

        apps.default = flake-utils.lib.mkApp {
          drv = noflex;
          name = "noflex-server";
        };

        devShells = {
          # Server, web and Flutter: what a plain `nix develop` lands in.
          default = pkgs.mkShell ({
              inputsFrom = [noflex];
              packages = taskPkgs ++ rustPkgs ++ webPkgs ++ flutterPkgs;
              buildInputs = flutterLinuxLibs;
              shellHook = rustShellHook + flutterShellHook;
            }
            // rustEnv
            // flutterAndroidEnv);

          # Server and web, without the Flutter and Android downloads.
          no-flutter = pkgs.mkShell ({
              inputsFrom = [noflex];
              packages = taskPkgs ++ rustPkgs ++ webPkgs;
              shellHook = rustShellHook;
            }
            // rustEnv);

          # Flutter and Android alone, for working on the apps only.
          flutter-only = pkgsFlutter.mkShell ({
              packages = taskPkgs ++ flutterPkgs;
              buildInputs = flutterLinuxLibs;
              shellHook = flutterShellHook;
            }
            // flutterAndroidEnv);

          # What CI's analysis and tests need: no Android at all.
          flutter-check = pkgsFlutter.mkShell ({
              packages = flutterBasePkgs;
              buildInputs = flutterLinuxLibs;
              shellHook = flutterShellHook;
            }
            // flutterEnv);

          # Enough to build for Android, without a device to run it on.
          flutter-android = pkgsFlutter.mkShell ({
              packages = flutterBasePkgs ++ [jdk androidBuildSdk];
              shellHook = flutterShellHook;
            }
            // flutterEnv
            // androidEnvFor androidBuildSdk);
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
