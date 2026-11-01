<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  let { channel, size = 40 } = $props();

  // CSP limits img-src to 'self', so remote M3U logos would only produce
  // console noise; fall back to a monogram for those.
  function usable(url) {
    if (!url) return false;
    try { return new URL(url, location.href).origin === location.origin; } catch { return false; }
  }

  let failed = $state(false);
  let src = $derived(!failed && usable(channel.logoUrl) ? channel.logoUrl : null);
  let initials = $derived((channel.name.match(/[A-Za-z0-9]+/g) ?? ["?"]).slice(0, 2).map((word) => word[0]).join("").toUpperCase());
  let hue = $derived([...channel.name].reduce((hash, char) => (hash * 31 + char.charCodeAt(0)) | 0, 0) % 360);
</script>

<span class="logo" class:image={src} style:--s="{size}px" style:--h={Math.abs(hue)} aria-hidden="true">
  {#if src}<img {src} alt="" loading="lazy" decoding="async" onerror={() => (failed = true)} />{:else}{initials}{/if}
</span>

<style>
  .logo { display: grid; place-items: center; width: var(--s); height: var(--s); flex: none; border-radius: calc(var(--s) * 0.28); overflow: hidden; font-size: calc(var(--s) * 0.34); font-weight: 750; letter-spacing: -0.02em; color: hsl(var(--h) 70% 82%); background: linear-gradient(135deg, hsl(var(--h) 45% 26%), hsl(calc(var(--h) + 40) 40% 16%)); box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.08); }
  .logo.image { background: rgb(255 255 255 / 0.92); }
  img { width: 100%; height: 100%; object-fit: contain; padding: 12%; }
</style>
