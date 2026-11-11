# Shared Code

`lib/` holds source more than one client imports, split by language. `lib/ts/` is the only one so far: Rust shares through `backend/`'s workspace, and Flutter is a single project.

## `lib/ts/`

- Consumed as source through an alias in each client's bundler and tsconfig, with no package or build step. `$codecs` is `lib/ts/codecSupport.ts`, `$devices` is `lib/ts/device.ts`, `$quality` is `lib/ts/quality.ts`.
- **Files here import nothing from a client**, and nothing that needs an install: `lib/ts/` has no `node_modules`, and each client must build it without the other's.
- Keeps its own `tsconfig.json` because bundlers pick a file's tsconfig by walking up from it. Each client type-checks what it imports under its own settings.
- `device.ts` says what a device is and leaves its icon to each client: `web/`'s `DeviceIcon`, `solidtv/`'s `BRAND_ICONS`. `flutter/lib/utils/device_icon.dart` ports its table, so a new brand goes in all three, and in the tests they each hold of that table.
- A new top-level input to the web build has to be added to the `web` fileset in `flake.nix`, or the release package can't see it.
- `quality.ts` is the quality menu's choice and Auto's policy (`AutoQuality`): when to leave the original for the server's ladder and when to go back, the one move no player's own ABR makes. `flutter/lib/features/player/quality.dart` ports it, so a threshold changes in both.
- `codecSupport.ts` probes through a `ProbeEnv`: this browser's media stack, or `probeElsewhere`, a device answering `MediaQuestion`s: the Cast receiver runs it on itself, bundled by `site/scripts/cast-receiver.js`.
