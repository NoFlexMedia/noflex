# Plugin Licensing

NoFlex itself is licensed under **AGPL-3.0-only**. This document explains
how that license applies (and doesn't apply) to plugins.

## TL;DR

> WASM plugins loaded by NoFlex are independent works. They may be
> distributed under **any license the author chooses**, including
> proprietary licenses. Loading a plugin does not extend NoFlex's AGPL
> terms to the plugin source.

## Why this is the project's position

NoFlex plugins are compiled to standalone WebAssembly components and
loaded by the NoFlex server through the WIT (WebAssembly Interface Types)
boundary. There is no source linkage, no shared address space at compile
time, and no static or dynamic linking against NoFlex's binary. Plugins
communicate with the server exclusively through capability-typed
function calls and streams.

In the language of the FSF's interpretation of the GPL: a plugin
communicating across this kind of boundary is more like a *separate
program that exchanges data* than a *derivative work*. The WIT contract
is the same kind of interface as a network protocol — and the FSF has
consistently treated programs that communicate over a network protocol
as independent works.

## What this means for plugin authors

- **You choose the license.** Apache-2.0, MIT, BSD, GPL, AGPL,
  proprietary — your choice.
- **You don't need to publish source** to distribute a plugin, unless
  your chosen license requires it.
- **You can sell plugins** or distribute them commercially.

## What this means for NoFlex operators

- **You can run plugins under any license** without that affecting your
  obligations under NoFlex's AGPL.
- The AGPL still applies to NoFlex itself: if you modify NoFlex and
  expose it as a network service, you must offer the modified NoFlex
  source.

## What this is NOT

This document is **not legal advice**. It is a good-faith statement of
how the project understands its own license boundary and intends to
apply it.

## Edge cases

- **Plugins that statically link NoFlex source.** If a plugin somehow
  pulls in NoFlex code as a Cargo dependency (rather than through the
  WIT boundary), AGPL terms apply to that plugin. This is unusual —
  the plugin SDK exposes WIT types, not NoFlex's domain crates.
- **Forks of NoFlex with a different plugin contract.** A fork that
  embeds plugins in-process (rather than across WASM) creates a
  different boundary; the analysis above is specific to the WASM
  plugin runtime.

## Questions

Open an issue on the repository.
