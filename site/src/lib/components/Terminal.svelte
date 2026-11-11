<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Copy from "phosphor-svelte/lib/Copy";

  let { lines = [], title = "Terminal", note = null } = $props();
  let copied = $state(false);
  let timer = 0;

  async function copy() {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 1800);
    } catch {
      // Clipboard access denied: the text is still there to select.
    }
  }
</script>

<div class="terminal">
  <div class="bar">
    <span class="lights"><i></i><i></i><i></i></span>
    <span class="title">{title}</span>
    <button class="copy" class:copied onclick={copy} aria-label={copied ? "Copied" : "Copy commands"}>
      {#if copied}<Check size={15} weight="bold" />Copied{:else}<Copy size={15} />Copy{/if}
    </button>
  </div>
  <pre><code>{#each lines as line}<span class="line"><span class="prompt" aria-hidden="true">$ </span>{line}</span>{"\n"}{/each}</code></pre>
  {#if note}<p class="note">{note}</p>{/if}
</div>

<style>
  .terminal { border-radius: var(--r-l); background: #0a0b0e; border: 1px solid var(--line-strong); box-shadow: var(--shadow-3); overflow: hidden; }
  .bar { display: flex; align-items: center; gap: 12px; height: 42px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--line); background: rgb(255 255 255 / 0.02); }
  .lights { display: flex; gap: 6px; }
  .lights i { width: 10px; height: 10px; border-radius: 50%; background: rgb(255 255 255 / 0.12); }
  .title { flex: 1; color: var(--text-3); font-size: 0.78rem; font-weight: 550; }
  .copy { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; border-radius: 999px; color: var(--text-2); font-size: 0.78rem; font-weight: 600; transition: background var(--t), color var(--t), transform var(--t-fast) var(--ease-out); }
  .copy:hover { background: var(--surface-2); color: var(--text); }
  .copy:active { transform: scale(0.94); }
  .copy.copied { color: var(--accent); }
  .copy.copied :global(svg) { animation: pop 420ms var(--ease-spring); }
  @keyframes pop { from { transform: scale(0.4); } }
  pre { margin: 0; padding: 18px 20px; overflow-x: auto; font-size: 0.92rem; line-height: 1.8; }
  code { font-size: inherit; }
  .prompt { color: var(--accent); user-select: none; }
  .note { padding: 0 20px 18px; color: var(--text-3); font-size: 0.84rem; }
</style>
