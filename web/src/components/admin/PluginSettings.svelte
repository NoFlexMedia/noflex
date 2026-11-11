<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowCounterClockwise from "phosphor-svelte/lib/ArrowCounterClockwise";
  import { graphql } from "$houdini";
  import { attempt } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Select from "$components/ui/Select.svelte";

  /** `plugin` is one of the Admin query's plugins. */
  let { open = $bindable(false), plugin } = $props();

  // The reloaded plugin comes back by id, so the Admin query repaints.
  const save = graphql(`
    mutation SetPluginSettings($plugin: String!, $values: [PluginSettingInput!]!) {
      setPluginSettings(pluginId: $plugin, values: $values) { id settings { ...PluginSettingFields } }
    }
  `);

  /** Changes by key: a string to set, null to clear. Untouched keys are absent. */
  let edits = $state({});
  let saving = $state(false);
  let settings = $derived(plugin?.settings ?? []);
  // A secret's empty field is one opened and left blank, not a clear.
  let values = $derived(
    Object.entries(edits)
      .filter(([key, value]) => !(value === "" && settings.find((s) => s.key === key)?.kind === "SECRET"))
      .map(([key, value]) => ({ key, value })),
  );

  $effect(() => {
    if (open) edits = {};
  });

  const SOURCE = { ADMIN: "Set here", CONFIG: "From config.toml", DEFAULT: "Default", UNSET: "Not set" };
  function status(setting) {
    const secret = setting.kind === "SECRET";
    if (!(setting.key in edits)) return secret && setting.source === "CONFIG" ? "From a secret file" : SOURCE[setting.source];
    if (edits[setting.key] === null) return secret ? "Clears on save" : "Falls back on save";
    return "Changed";
  }
  const current = (setting) => (setting.key in edits ? (edits[setting.key] ?? "") : (setting.value ?? ""));
  const edit = (setting, value) => (edits = { ...edits, [setting.key]: value });
  function undo(setting) {
    const { [setting.key]: _, ...rest } = edits;
    edits = rest;
  }
  function choices(setting) {
    const items = setting.choices.map((choice) => ({ value: choice, label: choice }));
    return setting.required ? items : [{ value: "", label: "Not set" }, ...items];
  }

  async function submit(event) {
    event.preventDefault();
    saving = true;
    const ok = await attempt(() => save.mutate({ plugin: plugin.id, values }), `${plugin.displayName} settings saved`);
    saving = false;
    if (ok) open = false;
  }
</script>

<Dialog bind:open title="{plugin?.displayName ?? 'Plugin'} settings" description="Saving reloads the plugin. A value set here wins over config.toml and secret files until you reset it." width={520}>
  <form id="plugin-settings" class="settings" onsubmit={submit}>
    {#each settings as setting (setting.key)}
      {@const id = `plugin-setting-${setting.key}`}
      {@const edited = setting.key in edits}
      {@const typing = edited && edits[setting.key] !== null}
      <div class="setting">
        <div class="head">
          <label for={id}>{setting.label}{#if setting.required}<span class="faint required">Required</span>{/if}</label>
          <span class="status" class:changed={edited}>{status(setting)}</span>
        </div>
        {#if setting.description}<small class="hint">{setting.description}</small>{/if}
        <div class="control">
          {#if setting.kind === "BOOLEAN"}
            <input {id} type="checkbox" class="switch" checked={current(setting) === "true"} onchange={(event) => edit(setting, String(event.currentTarget.checked))} />
          {:else if setting.kind === "CHOICE"}
            <div class="grow"><Select {id} value={current(setting)} items={choices(setting)} label={setting.label} placeholder="Not set" onchange={(value) => edit(setting, value)} /></div>
          {:else if setting.kind === "SECRET" && !typing}
            <button {id} type="button" class="btn sm" onclick={() => edit(setting, "")}>{setting.isSet ? "Replace" : "Set"}</button>
          {:else if setting.kind === "SECRET"}
            <!-- svelte-ignore a11y_autofocus -->
            <input {id} class="input grow" type="password" autocomplete="new-password" spellcheck="false" placeholder="New {setting.label.toLowerCase()}" value={edits[setting.key]} oninput={(event) => edit(setting, event.currentTarget.value)} autofocus />
          {:else}
            <input
              {id}
              class="input grow"
              type={setting.kind === "URL" ? "url" : "text"}
              inputmode={setting.kind === "INTEGER" ? "numeric" : undefined}
              spellcheck="false"
              value={current(setting)}
              placeholder={setting.default ?? ""}
              oninput={(event) => edit(setting, event.currentTarget.value)}
            />
          {/if}
          {#if edited}
            <button type="button" class="btn ghost sm" onclick={() => undo(setting)}>Undo</button>
          {:else if setting.source === "ADMIN"}
            <button type="button" class="btn ghost sm" title={setting.kind === "SECRET" ? "Clear the value set here" : "Use config.toml or the default again"} onclick={() => edit(setting, null)}><ArrowCounterClockwise size={14} />{setting.kind === "SECRET" ? "Clear" : "Reset"}</button>
          {/if}
        </div>
      </div>
    {:else}
      <p class="faint">This plugin has no settings.</p>
    {/each}
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (open = false)}>Cancel</button>
    <button class="btn primary" form="plugin-settings" disabled={saving || !values.length}>{#if saving}<span class="spinner sm"></span>{/if}Save</button>
  {/snippet}
</Dialog>

<style>
  .settings { display: grid; }
  .setting { display: grid; gap: 6px; padding: 14px 0; border-top: 1px solid var(--line); }
  .setting:first-child { border-top: 0; padding-top: 0; }
  .setting:last-child { padding-bottom: 0; }
  .head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 4px 12px; }
  .head label { font-size: 0.9rem; font-weight: 600; color: var(--text); }
  .required { margin-left: 8px; font-size: 0.78rem; font-weight: 500; }
  .status { font-size: 0.78rem; color: var(--text-3); white-space: nowrap; }
  .status.changed { color: var(--accent); }
  .hint { font-size: 0.82rem; color: var(--text-3); }
  .control { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 2px; }
  .grow { flex: 1 1 220px; min-width: 0; }
</style>
