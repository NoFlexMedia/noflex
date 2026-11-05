<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import BookmarkSimple from "phosphor-svelte/lib/BookmarkSimple";
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import Check from "phosphor-svelte/lib/Check";
  import Plus from "phosphor-svelte/lib/Plus";
  import SlidersHorizontal from "phosphor-svelte/lib/SlidersHorizontal";
  import Trash from "phosphor-svelte/lib/Trash";
  import X from "phosphor-svelte/lib/X";
  import { Popover } from "bits-ui";
  import { fade, slide } from "svelte/transition";
  import { page } from "$app/state";
  import { graphql } from "$houdini";
  import { FIELDS, fieldOf, operatorOf, valueLabel } from "$lib/filterFields.js";
  import { fetchQuery } from "$lib/graphql.js";
  import { servers } from "$lib/servers.svelte.js";
  import { app, SAVED_FILTERS_KEY } from "$lib/session.svelte.js";
  import { attempt, prefersReducedMotion } from "$lib/ui.svelte.js";
  import { newGroup, newRule, pruned, ruleValues, serializeRules } from "$lib/workFilters.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import { describeTree } from "./describe.js";
  import FacetPicker from "./FacetPicker.svelte";
  import RuleGroup from "./RuleGroup.svelte";
  import { RuleEditor } from "./ruleEditor.svelte.js";

  /**
   * Simple filters, with the full rule builder as an advanced mode over the same
   * rules. `scope` is `{ library }` or `{ category }`: which works' values to
   * offer. `update` writes URL parameters. `extra` renders beside the pills;
   * `more` at the end of the "More" menu.
   */
  let { scope, update, extra = null, more = null } = $props();

  const editor = new RuleEditor((serialized) => update({ rules: serialized }));
  let urlRules = $derived(page.url.searchParams.get("rules") ?? "");
  $effect(() => { editor.sync(urlRules); });

  // ---- What the server can offer. Walks every file, so it waits until it's wanted. ----
  const facets = graphql(`
    query WorkFacets($library: ID, $category: ContentCategory) @cache(policy: NetworkOnly) {
      workFacets(libraryId: $library, category: $category) { facet truncated values { value label workCount } }
    }
  `);
  let wanted = $state(false);
  let hasRules = $derived(!!urlRules);
  $effect(() => {
    if (!wanted && !hasRules) return;
    // Follows the active server: a linked library's values are that server's.
    void servers.active;
    fetchQuery(facets, { library: scope.library ?? null, category: scope.category ?? null });
  });
  let facetMap = $derived(new Map(($facets.data?.workFacets ?? []).map((entry) => [entry.facet, entry])));
  let loading = $derived($facets.fetching);
  const nameOf = (facet, value) => facetMap.get(facet)?.values.find((option) => option.value === value)?.label;

  // ---- Which mode can show these rules ----
  let advanced = $state(false);
  let meant = $derived(pruned(editor.tree));
  // Pills say "is any of" and hold one condition per field; anything else needs the builder to read.
  let isSimple = $derived(!meant || (meant.match === "ALL" && meant.items.every((item, index) => item.kind === "rule" && item.op === fieldOf(item.facet).operators[0].id && meant.items.findIndex((other) => other.facet === item.facet) === index)));
  let sentence = $derived(describeTree(editor.tree, nameOf));

  function openAdvanced(next = !advanced) {
    advanced = next;
    if (next) wanted = true;
  }

  // ---- Simple filters ----
  let added = $state([]);
  let openPill = $state("");
  let pills = $derived(FIELDS.filter((field) => field.quick || added.includes(field.facet) || (meant?.items ?? []).some((item) => item.facet === field.facet)));
  let moreFields = $derived(FIELDS.filter((field) => !pills.includes(field)));
  let moreOpen = $state(false);
  // Follows the thumb; the rule only changes where it lands, not on every step of the way.
  let ratingPreview = $state(null);
  const ruleFor = (facet) => editor.tree.items.find((item) => item.kind === "rule" && item.facet === facet);
  const inputsOf = (facet) => ruleFor(facet)?.inputs ?? [];
  // A condition the builder hasn't had filled in yet isn't filtering anything.
  const isFiltering = (facet) => { const rule = ruleFor(facet); return !!rule && ruleValues(rule).length > 0; };

  function setSimple(facet, inputs) {
    const rule = ruleFor(facet);
    if (rule && inputs.length) editor.setInputs(rule, inputs);
    else if (rule) editor.remove(rule);
    else if (inputs.length) {
      const fresh = newRule(facet);
      editor.tree.items.push(fresh);
      editor.setInputs(editor.tree.items.at(-1), inputs);
    }
  }
  function pillText(field) {
    if (!isFiltering(field.facet)) return field.pill ?? field.label;
    const rule = ruleFor(field.facet);
    const operator = operatorOf(field, rule.op);
    if (operator.input === "number") return `★ ${rule.inputs[0]}+`;
    if (operator.input === "days") return `Added in ${rule.inputs[0]} days`;
    if (operator.input === "text") return `“${rule.inputs[0]}”`;
    if (operator.input === "none") return field.label;
    const names = rule.inputs.map((value) => valueLabel(field, value, nameOf(field.facet, value)));
    return names.length > 1 ? `${names[0]} +${names.length - 1}` : names[0];
  }
  function addPill(field) {
    moreOpen = false;
    wanted = true;
    if (operatorOf(field, field.operators[0].id).input === "none") { setSimple(field.facet, field.operators[0].encode([])); return; }
    added = [...added, field.facet];
    // After the menu has closed and given focus back, or the new pill loses it again.
    requestAnimationFrame(() => (openPill = field.facet));
  }
  function clearPill(field) {
    setSimple(field.facet, []);
    added = added.filter((facet) => facet !== field.facet);
  }

  // ---- Starting points, which double as examples of what the builder can say ----
  const rule = (facet, ...inputs) => newRule(facet, undefined, inputs);
  const TEMPLATES = [
    { name: "Unwatched in 4K", tree: () => newGroup("ALL", [rule("WATCH_STATE", "unwatched"), rule("RESOLUTION", "4k", "8k")]) },
    { name: "Great, and not seen yet", tree: () => newGroup("ALL", [rule("MIN_RATING", "8"), rule("WATCH_STATE", "unwatched")]) },
    { name: "Home theater night", tree: () => newGroup("ALL", [rule("DYNAMIC_RANGE", "dolby_vision", "hdr10"), rule("AUDIO_LAYOUT", "atmos", "7.1")]) },
    { name: "Added this month", tree: () => newGroup("ALL", [rule("ADDED_WITHIN_DAYS", "30")]) },
  ];

  // ---- Saved filters ----
  const saveFilters = graphql(`
    mutation SaveBrowseFilters($key: String!, $value: JSON!) { setPreference(key: $key, value: $value) { key } }
  `);
  let saving = $state(false);
  let saveName = $state("");
  let managing = $state(false);
  const persist = (list, done) => attempt(() => saveFilters.mutate({ key: SAVED_FILTERS_KEY, value: list }, { metadata: { ownServer: true } }), done).then((result) => {
    if (result) app.savedFilters = list;
    return result;
  });
  async function save(event) {
    event.preventDefault();
    const name = saveName.trim();
    if (!name) return;
    const list = [...app.savedFilters.filter((item) => item.name !== name), { name, rules: serializeRules(editor.tree) }];
    if (await persist(list, `Saved “${name}”`)) saving = false;
  }
  function openSave() {
    saveName = "";
    saving = true;
  }

  let motion = $derived(prefersReducedMotion() ? 0 : 240);
</script>

<div class="bar">
  {#if isSimple}
    <div class="pills" in:fade={{ duration: motion / 2 }}>
      {#each pills as field (field.facet)}
        {@const active = isFiltering(field.facet)}
        {@const operator = operatorOf(field, field.operators[0].id)}
        <span class="pill-wrap" class:active transition:fade={{ duration: motion / 2 }}>
          {#if operator.input === "none"}
            <button class="pill" aria-pressed={active} onclick={() => (active ? clearPill(field) : setSimple(field.facet, operator.encode([])))}>{field.label}</button>
          {:else}
            <Popover.Root open={openPill === field.facet} onOpenChange={(next) => { openPill = next ? field.facet : ""; if (next) wanted = true; }}>
              <Popover.Trigger class="pill">{pillText(field)}{#if !active}<CaretDown size={13} />{/if}</Popover.Trigger>
              <Popover.Portal>
                <Popover.Content class="nf-menu picker-pop" sideOffset={8} align="start" collisionPadding={12}>
                  {#if field.hint}<p class="pop-hint">{field.hint}</p>{/if}
                  {#if operator.input === "pick"}
                    <FacetPicker {field} options={facetMap.get(field.facet)?.values} truncated={facetMap.get(field.facet)?.truncated} {loading} selected={inputsOf(field.facet)} onchange={(values) => setSimple(field.facet, values)} />
                  {:else if operator.input === "number"}
                    <label class="range">
                      <span>At least <strong class="tabular">{ratingPreview ?? inputsOf(field.facet)[0] ?? "—"}</strong> out of 10</span>
                      <input type="range" min="1" max="10" step="0.5" value={inputsOf(field.facet)[0] ?? 7} aria-label="Minimum rating" oninput={(event) => (ratingPreview = event.currentTarget.value)} onchange={(event) => { ratingPreview = null; setSimple(field.facet, [event.currentTarget.value]); }} />
                    </label>
                  {:else if operator.input === "text"}
                    <!-- svelte-ignore a11y_autofocus -->
                    <input class="input" autofocus value={inputsOf(field.facet)[0] ?? ""} placeholder="Words in the title" aria-label="Words in the title" onchange={(event) => setSimple(field.facet, event.currentTarget.value.trim() ? [event.currentTarget.value.trim()] : [])} />
                  {:else if operator.input === "days"}
                    <label class="range"><span>Added in the last</span><span class="days"><input class="input" type="number" min="1" value={inputsOf(field.facet)[0] ?? ""} placeholder="30" onchange={(event) => setSimple(field.facet, event.currentTarget.value ? [event.currentTarget.value] : [])} /> days</span></label>
                  {/if}
                </Popover.Content>
              </Popover.Portal>
            </Popover.Root>
          {/if}
          {#if active}<button class="pill-clear" aria-label="Clear {field.label.toLowerCase()}" onclick={() => clearPill(field)}><X size={12} weight="bold" /></button>{/if}
        </span>
      {/each}

      <Popover.Root bind:open={moreOpen}>
        <Popover.Trigger class="pill quiet"><Plus size={13} />More</Popover.Trigger>
        <Popover.Portal>
          <Popover.Content class="nf-menu more-pop" sideOffset={8} align="start" collisionPadding={12}>
            {#each [...new Set(moreFields.map((field) => field.section))] as section}
              <div class="nf-menu-label">{section}</div>
              <div class="more-grid">
                {#each moreFields.filter((field) => field.section === section) as field (field.facet)}
                  <button class="more-item" onclick={() => addPill(field)}>{field.label}</button>
                {/each}
              </div>
            {/each}
            {#if more}<div class="nf-menu-sep"></div>{@render more()}{/if}
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
      {@render extra?.()}
    </div>
  {:else if !advanced}
    <div class="custom" in:fade={{ duration: motion / 2 }}>
      <SlidersHorizontal size={16} />
      <p>{sentence}</p>
      <button class="btn ghost sm" onclick={() => openAdvanced(true)}>Edit rules</button>
    </div>
  {:else}
    <div class="pills">{@render extra?.()}</div>
  {/if}

  <div class="ends">
    {#if meant}<button class="btn ghost sm" in:fade={{ duration: motion / 2 }} onclick={() => { added = []; editor.clear(); }}>Clear</button>{/if}
    {#if app.savedFilters.length}
      <Menu width={240}>
        {#snippet trigger(props)}<button {...props} class="btn ghost sm"><BookmarkSimple size={15} />Saved</button>{/snippet}
        {#each app.savedFilters as item (item.name)}
          <MenuItem icon={item.rules === urlRules ? Check : null} onSelect={() => editor.replace(item.rules)}>{item.name}</MenuItem>
        {/each}
        <div class="nf-menu-sep"></div>
        <MenuItem onSelect={() => (managing = true)}>Manage saved filters…</MenuItem>
      </Menu>
    {/if}
    <button class="btn sm {advanced ? 'outline' : 'ghost'}" aria-expanded={advanced} onclick={() => openAdvanced()}><SlidersHorizontal size={15} />Advanced</button>
  </div>
</div>

{#if advanced}
  <section class="builder" transition:slide={{ duration: motion }} aria-label="Advanced filters">
    <header>
      <p class="reads" aria-live="polite">
        {#if sentence}<span class="faint">Showing titles where</span> {sentence}.{:else}<span class="faint">Combine as many conditions as you like, and use a group when part of the filter follows its own logic, like “any of” inside “all of”. Results update as you go.</span>{/if}
      </p>
      <div class="head-actions">
        {#if meant}<button class="btn sm" onclick={openSave}><BookmarkSimple size={15} />Save as…</button>{/if}
        <button class="icon-btn sm" aria-label="Close advanced filters" onclick={() => openAdvanced(false)}><X size={16} /></button>
      </div>
    </header>

    <RuleGroup group={editor.tree} {editor} facets={facetMap} {loading} />

    {#if !editor.tree.items.length}
      <div class="starters" transition:fade={{ duration: motion / 2 }}>
        <span class="faint">Or start from one of these:</span>
        {#each TEMPLATES as template (template.name)}
          <button class="pill" onclick={() => editor.replace(serializeRules(template.tree()))}>{template.name}</button>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<Dialog bind:open={saving} title="Save this filter" description="It appears under Saved everywhere you browse, on your account." width={420}>
  <form id="save-filter" onsubmit={save}>
    <label class="field">Name<input bind:value={saveName} required maxlength="60" placeholder="Home theater night" /></label>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (saving = false)}>Cancel</button>
    <button class="btn primary" form="save-filter" disabled={!saveName.trim()}>Save</button>
  {/snippet}
</Dialog>

<Dialog bind:open={managing} title="Saved filters" width={420}>
  <ul class="list">
    {#each app.savedFilters as item (item.name)}
      <li>
        <span class="saved-name">{item.name}</span>
        <button class="icon-btn sm" aria-label="Delete {item.name}" onclick={() => persist(app.savedFilters.filter((other) => other !== item), `Deleted “${item.name}”`)}><Trash size={15} /></button>
      </li>
    {:else}
      <p class="empty-note">Nothing saved. Build a filter under Advanced, then save it from there.</p>
    {/each}
  </ul>
  {#snippet footer()}<button class="btn ghost" onclick={() => (managing = false)}>Done</button>{/snippet}
</Dialog>

<style>
  .bar { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px 16px; margin: -10px 0 22px; }
  .pills { flex: 1; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; min-width: 0; }
  .ends { flex: none; display: flex; align-items: center; gap: 4px; }
  .pill-wrap { position: relative; display: inline-flex; align-items: center; border-radius: 99px; transition: background var(--t-fast); }
  .pill-wrap :global(.pill), .pill, .pills :global(.pill) { display: inline-flex; align-items: center; gap: 6px; height: 32px; max-width: 220px; padding: 0 12px; border: 1px solid var(--line); border-radius: 99px; color: var(--text-2); font-size: 0.84rem; font-weight: 550; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; transition: border-color var(--t-fast), color var(--t-fast), background var(--t-fast), transform var(--t) var(--ease-spring); }
  .pills :global(.pill:hover), .starters .pill:hover { border-color: var(--line-strong); color: var(--text); }
  .pills :global(.pill:active), .starters .pill:active { transform: scale(0.95); }
  .pills :global(.pill.quiet) { border-style: dashed; color: var(--text-3); }
  .pill-wrap.active :global(.pill) { padding-right: 30px; border-color: rgb(212 255 90 / 0.4); background: var(--accent-soft); color: var(--text); }
  .pill-clear { position: absolute; right: 6px; top: 50%; translate: 0 -50%; display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; color: var(--text-2); transition: background var(--t-fast), color var(--t-fast); }
  .pill-clear:hover { background: var(--surface-3); color: var(--text); }
  :global(.picker-pop) { width: min(300px, calc(100vw - 24px)); padding: 10px !important; }
  :global(.more-pop) { width: min(340px, calc(100vw - 24px)); padding: 6px 10px 10px !important; }
  .pop-hint { padding: 2px 4px 8px; font-size: 0.78rem; line-height: 1.4; color: var(--text-3); }
  .more-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1px; }
  .more-item { padding: 7px 10px; border-radius: 9px; font-size: 0.88rem; text-align: left; color: var(--text-2); transition: background var(--t-fast), color var(--t-fast); }
  .more-item:hover { background: var(--surface-2); color: var(--text); }
  .range { display: grid; gap: 10px; padding: 6px 4px; font-size: 0.88rem; color: var(--text-2); }
  .range input[type="range"] { width: 100%; accent-color: var(--accent); }
  .days { display: flex; align-items: center; gap: 8px; }
  .custom { flex: 1; display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 40px; padding: 4px 6px 4px 14px; border: 1px solid var(--line); border-radius: var(--r); color: var(--text-3); }
  .custom p { flex: 1; min-width: 0; color: var(--text-2); font-size: 0.88rem; line-height: 1.4; }
  .builder { display: grid; gap: 16px; margin: -8px 0 26px; padding: 18px; border: 1px solid var(--line); border-radius: var(--r-l); background: var(--surface); }
  .builder > header { display: flex; align-items: flex-start; justify-content: space-between; gap: 14px; }
  .reads { max-width: 72ch; font-size: 0.9rem; line-height: 1.45; color: var(--text); }
  .head-actions { flex: none; display: flex; align-items: center; gap: 6px; }
  .starters { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 0.84rem; }
  .saved-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 720px) { .bar { flex-direction: column; } .ends { align-self: flex-end; } .builder { padding: 14px; } }
</style>
