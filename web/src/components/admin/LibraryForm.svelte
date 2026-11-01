<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { untrack } from "svelte";
  import { CreateLibraryStore, UpdateLibraryConfigStore, UpdateLibraryStore } from "$houdini";
  import { splitList, titleCase } from "$lib/format.js";
  import { attempt } from "$lib/ui.svelte.js";
  import ArrowDown from "phosphor-svelte/lib/ArrowDown";
  import ArrowUp from "phosphor-svelte/lib/ArrowUp";
  import Plus from "phosphor-svelte/lib/Plus";
  import X from "phosphor-svelte/lib/X";
  import Dialog from "$components/ui/Dialog.svelte";
  import Select from "$components/ui/Select.svelte";

  /** `library` null → create (centered dialog); otherwise edit (sheet). */
  let { open = $bindable(false), library = null, providers = [], onsaved } = $props();

  const KINDS = ["MOVIES", "SHOWS", "MUSIC", "AUDIOBOOKS", "PHOTOS", "MIXED"].map((value) => ({ value, label: titleCase(value) }));
  const SCHEDULES = [
    { value: "DISABLED", label: "Manual only" },
    { value: "HOURLY", label: "Every hour" },
    { value: "DAILY", label: "Daily" },
    { value: "WEEKLY", label: "Weekly" },
  ];
  const ARTWORK = [
    { value: "PREFER_PROVIDER", label: "From metadata providers" },
    { value: "PREFER_LOCAL", label: "From the media folders" },
  ];

  const createLibrary = new CreateLibraryStore();
  const updateLibrary = new UpdateLibraryStore();
  const updateConfig = new UpdateLibraryConfigStore();

  let form = $state(blank());
  let saving = $state(false);

  function blank() {
    return { name: "", kind: "MOVIES", paths: "", languages: "", providers: [], excluded: "", schedule: "DAILY", watch: true, artwork: "PREFER_PROVIDER", scan: true };
  }

  $effect(() => {
    if (!open) return;
    // Seed once per open; a background refresh mustn't clobber edits.
    form = untrack(() => library
      ? { name: library.name, kind: library.kind, paths: library.paths.join("\n"), languages: library.config.preferredLanguages.join(", "), providers: [...library.config.preferredProviders], excluded: library.config.excludedFolders.join("\n"), schedule: library.config.scanSchedule, watch: library.config.watchEnabled, artwork: library.config.artworkPreference, scan: false }
      : blank());
  });

  let chosen = $derived(form.providers);
  let available = $derived(providers.filter((id) => !chosen.includes(id)));
  const addProvider = (id) => { form.providers = [...chosen, id]; };
  const removeProvider = (index) => { form.providers = chosen.filter((_, i) => i !== index); };
  function moveProvider(index, by) {
    const next = [...chosen];
    [next[index], next[index + by]] = [next[index + by], next[index]];
    form.providers = next;
  }
  const same = (a, b) => a.length === b.length && a.every((item, index) => item === b[index]);

  async function submit(event) {
    event.preventDefault();
    const paths = splitList(form.paths);
    const config = { languages: splitList(form.languages), providers: [...chosen], excluded: splitList(form.excluded), watch: form.watch, schedule: form.schedule, artwork: form.artwork };
    saving = true;
    let result;
    if (!library) {
      // Two steps; once the library exists, a config failure mustn't invite a duplicate create.
      result = (await attempt(() => createLibrary.mutate({ name: form.name.trim(), kind: form.kind, paths }), `“${form.name.trim()}” created`))?.data.createLibrary;
      if (result) result = (await attempt(() => updateConfig.mutate({ id: result.id, ...config })))?.data.updateLibraryConfig ?? result;
    } else {
      result = await attempt(async () => {
        let next = library;
        if (form.name.trim() !== library.name || form.kind !== library.kind || !same(paths, library.paths)) {
          next = (await updateLibrary.mutate({ id: library.id, name: form.name.trim(), kind: form.kind, paths })).data.updateLibrary;
        }
        const c = library.config;
        if (!same(config.languages, c.preferredLanguages) || !same(config.providers, c.preferredProviders) || !same(config.excluded, c.excludedFolders) || config.watch !== c.watchEnabled || config.schedule !== c.scanSchedule || config.artwork !== c.artworkPreference) {
          next = (await updateConfig.mutate({ id: library.id, ...config })).data.updateLibraryConfig;
        }
        return next;
      }, "Library saved");
    }
    saving = false;
    if (!result) return;
    open = false;
    onsaved?.(result, { scan: !library && form.scan });
  }
</script>

<Dialog bind:open title={library ? `Edit “${library.name}”` : "New library"} description={library ? "Folders, scanner behaviour and metadata providers." : "Point NoFlex at one or more folders of media."} variant={library ? "sheet" : "center"} width={library ? 560 : 540}>
  <form id="library-form" class="form" onsubmit={submit}>
    <fieldset>
      <legend>Basics</legend>
      <div class="two">
        <label class="field">Name<input bind:value={form.name} required placeholder="Films" autocomplete="off" /></label>
        <label class="field">Type<Select bind:value={form.kind} items={KINDS} label="Library type" /></label>
      </div>
      <label class="field">Folders<textarea bind:value={form.paths} required rows="3" placeholder={"/srv/media/films\n/mnt/archive/films"} spellcheck="false" class="mono"></textarea><small>One absolute path per line, as the server sees it.</small></label>
    </fieldset>

    <fieldset>
      <legend>Scanning</legend>
      <div class="two">
        <label class="field">Scheduled rescans<Select bind:value={form.schedule} items={SCHEDULES} label="Scan schedule" /></label>
        <label class="toggle"><input type="checkbox" class="switch" bind:checked={form.watch} /><span><strong>Watch folders</strong><small>Pick up changes as they happen</small></span></label>
      </div>
      <label class="field">Excluded folders<textarea bind:value={form.excluded} rows="2" placeholder={"incoming/.staging\nfailed"} spellcheck="false" class="mono"></textarea><small>Relative to each folder above; skipped recursively.</small></label>
    </fieldset>

    <fieldset>
      <legend>Metadata</legend>
      <div class="field">
        <span id="provider-order">Provider order</span>
        {#if chosen.length}
          <ol class="providers" aria-labelledby="provider-order">
            {#each chosen as id, index (id)}
              <li>
                <span class="rank tabular">{index + 1}</span>
                <span class="id" class:missing={!providers.includes(id)} title={providers.includes(id) ? undefined : "Not installed"}>{id}</span>
                <button type="button" class="icon-btn sm" disabled={index === 0} onclick={() => moveProvider(index, -1)} aria-label="Move {id} up"><ArrowUp size={15} /></button>
                <button type="button" class="icon-btn sm" disabled={index === chosen.length - 1} onclick={() => moveProvider(index, 1)} aria-label="Move {id} down"><ArrowDown size={15} /></button>
                <button type="button" class="icon-btn sm" onclick={() => removeProvider(index)} aria-label="Remove {id}"><X size={15} /></button>
              </li>
            {/each}
          </ol>
        {:else}
          <p class="none">None chosen: every installed provider is tried, in its default priority.</p>
        {/if}
        {#if available.length}
          <div class="suggest" role="group" aria-label="Add a metadata provider">
            {#each available as id (id)}<button type="button" class="chip outline" onclick={() => addProvider(id)}><Plus size={12} weight="bold" />{id}</button>{/each}
          </div>
        {/if}
        <small>Only these providers are used, in this order. The first that matches a title supplies all of its metadata; the next is tried only if it fails.</small>
      </div>
      <label class="field">Preferred languages<input bind:value={form.languages} placeholder="en, ja" spellcheck="false" autocomplete="off" /><small>BCP-47 tags, most preferred first.</small></label>
      <label class="field">Artwork<Select bind:value={form.artwork} items={ARTWORK} label="Artwork source" /><small>Posters and backdrops to show first; the other source fills any gaps.</small></label>
    </fieldset>

    {#if !library}<label class="check"><input type="checkbox" bind:checked={form.scan} />Start the first scan right away</label>{/if}
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (open = false)}>Cancel</button>
    <button class="btn primary" form="library-form" disabled={saving || !form.name.trim() || !form.paths.trim()}>{#if saving}<span class="spinner sm"></span>{/if}{library ? "Save changes" : "Create library"}</button>
  {/snippet}
</Dialog>

<style>
  .form { display: grid; gap: 22px; }
  fieldset { display: grid; gap: 14px; margin: 0; padding: 0; border: 0; min-width: 0; }
  legend { padding: 0; margin-bottom: 12px; font-size: 0.72rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; align-items: end; }
  textarea.mono { font-size: 0.86rem; }
  .toggle { display: flex; align-items: center; gap: 12px; min-height: 42px; cursor: pointer; }
  .toggle span { display: grid; }
  .toggle strong { font-size: 0.9rem; font-weight: 600; }
  .toggle small { color: var(--text-3); font-size: 0.8rem; }
  .providers { display: grid; gap: 4px; margin: 0; padding: 0; list-style: none; }
  .providers li { display: flex; align-items: center; gap: 4px; padding: 4px 4px 4px 12px; border-radius: var(--r-s); background: var(--surface); border: 1px solid var(--line); }
  .providers .icon-btn:disabled { opacity: 0.3; cursor: default; }
  .rank { width: 18px; color: var(--text-3); font-size: 0.8rem; }
  .id { flex: 1; min-width: 0; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.86rem; }
  .id.missing { color: var(--text-3); text-decoration: line-through; }
  .none { margin: 0; color: var(--text-3); font-size: 0.86rem; }
  .suggest { display: flex; flex-wrap: wrap; gap: 6px; }
  .suggest .chip { display: inline-flex; align-items: center; gap: 4px; }
  .suggest .chip { cursor: pointer; font-family: ui-monospace, "SF Mono", Menlo, monospace; transition: border-color var(--t), color var(--t); }
  .suggest .chip:hover { border-color: var(--text-3); color: var(--text); }
  @media (max-width: 520px) { .two { grid-template-columns: 1fr; } }
</style>
