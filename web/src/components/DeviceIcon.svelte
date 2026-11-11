<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { deviceBrand } from "$devices";
  import AndroidLogo from "phosphor-svelte/lib/AndroidLogo";
  import AppleLogo from "phosphor-svelte/lib/AppleLogo";
  import Browser from "phosphor-svelte/lib/Browser";
  import Desktop from "phosphor-svelte/lib/Desktop";
  import DeviceMobile from "phosphor-svelte/lib/DeviceMobile";
  import DeviceTablet from "phosphor-svelte/lib/DeviceTablet";
  import GameController from "phosphor-svelte/lib/GameController";
  import Laptop from "phosphor-svelte/lib/Laptop";
  import LinuxLogo from "phosphor-svelte/lib/LinuxLogo";
  import Television from "phosphor-svelte/lib/Television";
  import WindowsLogo from "phosphor-svelte/lib/WindowsLogo";
  import { siArchlinux, siBrave, siDebian, siDuckduckgo, siFedora, siFirefoxbrowser, siGooglechrome, siGoogletv, siLinuxmint, siManjaro, siNixos, siNvidia, siOpensuse, siOpera, siPlaystation, siPopos, siSafari, siSteamdeck, siUbuntu, siVivaldi } from "simple-icons";

  /** `device` and `client` are the free text a session or registered device reports. Decorative: its name is beside it. */
  let { device = null, client = null, size = 18 } = $props();

  const MARKS = { archlinux: siArchlinux, brave: siBrave, chrome: siGooglechrome, debian: siDebian, duckduckgo: siDuckduckgo, fedora: siFedora, firefox: siFirefoxbrowser, googletv: siGoogletv, linuxmint: siLinuxmint, manjaro: siManjaro, nixos: siNixos, nvidia: siNvidia, opensuse: siOpensuse, opera: siOpera, playstation: siPlaystation, popos: siPopos, safari: siSafari, steamdeck: siSteamdeck, ubuntu: siUbuntu, vivaldi: siVivaldi };
  // Phosphor's own logos where Simple Icons has none or draws too much for this size,
  // filled to sit with its solid marks. What isn't a brand stays an outline.
  const LOGOS = { android: AndroidLogo, appletv: AppleLogo, linux: LinuxLogo, mac: AppleLogo, windows: WindowsLogo };
  const OUTLINES = { browser: Browser, desktop: Desktop, gamepad: GameController, laptop: Laptop, phone: DeviceMobile, tablet: DeviceTablet, television: Television };

  let brand = $derived(deviceBrand(device, client));
  let Logo = $derived(LOGOS[brand]);
  let Outline = $derived(OUTLINES[brand]);
</script>

{#if MARKS[brand]}
  <svg viewBox="0 0 24 24" width={size} height={size} fill="currentColor" aria-hidden="true"><path d={MARKS[brand].path} /></svg>
{:else if Logo}
  <Logo {size} weight="fill" />
{:else}
  <Outline {size} />
{/if}
