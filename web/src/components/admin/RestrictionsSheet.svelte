<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { graphql, ParentalTagsStore } from "$houdini";
  import { splitList } from "$lib/format.js";
  import { fetchQuery, request } from "$lib/graphql.js";
  import { attempt } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";

  let { open = $bindable(false), user = null } = $props();

  const ratings = new ParentalTagsStore();
  $effect(() => { if (open) fetchQuery(ratings); });
  let tags = $derived(($ratings.data?.tags.nodes ?? []).map((item) => item.tag));
  const restrictions = graphql(`
    query UserRestrictions($user: ID!) @cache(policy: NetworkOnly) {
      userRestrictions(userId: $user) { userId allowedParentalRatingIds allowUnrated maxStreamBitrateBps maxConcurrentTranscodes }
    }
  `);
  const setRestrictions = graphql(`
    mutation SetUserRestrictions($user: ID!, $input: UserRestrictionsInput!) {
      setUserRestrictions(userId: $user, input: $input) { userId allowUnrated maxStreamBitrateBps maxConcurrentTranscodes allowedParentalRatingIds }
    }
  `);

  let form = $state(null);
  let error = $state("");
  let saving = $state(false);

  // Keyed on the id so a background refresh of the user list doesn't wipe edits.
  let userId = $derived(user?.id);
  $effect(() => {
    if (!open || !userId) return;
    const id = userId;
    form = null; error = "";
    request(restrictions, { user: id })
      .then((data) => {
        if (userId !== id) return;
        const r = data.userRestrictions;
        form = {
          ids: [...r.allowedParentalRatingIds],
          extra: "",
          allowUnrated: r.allowUnrated,
          bitrate: r.maxStreamBitrateBps ? String(Math.round(r.maxStreamBitrateBps / 100_000) / 10) : "",
          transcodes: r.maxConcurrentTranscodes != null ? String(r.maxConcurrentTranscodes) : "",
        };
      })
      .catch((cause) => { error = cause.message; });
  });

  let known = $derived(new Set(tags.map((tag) => tag.id)));
  let orphans = $derived((form?.ids ?? []).filter((id) => !known.has(id)));

  function toggle(id) {
    form.ids = form.ids.includes(id) ? form.ids.filter((item) => item !== id) : [...form.ids, id];
  }

  async function save(event) {
    event.preventDefault();
    const input = {
      allowedParentalRatingIds: [...new Set([...form.ids, ...splitList(form.extra)])],
      allowUnrated: form.allowUnrated,
      maxStreamBitrateBps: form.bitrate ? Math.round(Number(form.bitrate) * 1_000_000) : null,
      maxConcurrentTranscodes: form.transcodes ? Number(form.transcodes) : null,
    };
    saving = true;
    const result = await attempt(() => setRestrictions.mutate({ user: user.id, input }), `Restrictions saved for ${user.name}`);
    saving = false;
    if (result) open = false;
  }
</script>

<Dialog bind:open title="Restrictions for {user?.name ?? ''}" description="Limits apply on every device this person signs in from." variant="sheet" width={520}>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if !form}
    <div class="stack" aria-busy="true">{#each Array(4) as _}<div class="skeleton line"></div>{/each}</div>
  {:else}
    <form id="restrictions-form" class="form" onsubmit={save}>
      <fieldset>
        <legend>Content ratings</legend>
        <p class="hint">{form.ids.length ? `Only titles rated ${form.ids.length === 1 ? "this way" : "one of these ways"} are visible.` : "Nothing selected — every rating is allowed."}</p>
        {#if ratings.loading}
          <div class="skeleton chips-skel"></div>
        {:else if tags.length}
          <div class="ratings" role="group" aria-label="Allowed ratings">
            {#each tags as tag (tag.id)}
              <button type="button" class="rating" class:on={form.ids.includes(tag.id)} aria-pressed={form.ids.includes(tag.id)} onclick={() => toggle(tag.id)}>{tag.display}</button>
            {/each}
          </div>
        {:else}
          <p class="hint">No ratings have been tagged in your libraries yet.</p>
        {/if}
        {#if orphans.length}
          <p class="hint">Also allowed: {orphans.length} rating {orphans.length === 1 ? "ID" : "IDs"} not found in any library. <button type="button" class="link" onclick={() => (form.ids = form.ids.filter((id) => known.has(id)))}>Remove</button></p>
        {/if}
        <details>
          <summary>Add rating IDs manually</summary>
          <label class="field">Tag IDs<textarea class="mono" bind:value={form.extra} rows="2" placeholder="One UUID per line" spellcheck="false"></textarea></label>
        </details>
        <label class="toggle"><input type="checkbox" class="switch" bind:checked={form.allowUnrated} /><span><strong>Allow unrated titles</strong><small>Titles without any rating tag</small></span></label>
      </fieldset>
      <fieldset>
        <legend>Streaming</legend>
        <div class="two">
          <label class="field">Max bitrate<span class="suffix"><input type="number" min="0.5" step="0.5" inputmode="decimal" bind:value={form.bitrate} placeholder="No cap" /><span>Mbps</span></span></label>
          <label class="field">Simultaneous transcodes<input type="number" min="0" step="1" inputmode="numeric" bind:value={form.transcodes} placeholder="Server default" /></label>
        </div>
      </fieldset>
    </form>
  {/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (open = false)}>Cancel</button>
    <button class="btn primary" form="restrictions-form" disabled={!form || saving}>{#if saving}<span class="spinner sm"></span>{/if}Save restrictions</button>
  {/snippet}
</Dialog>

<style>
  .form { display: grid; gap: 24px; }
  fieldset { display: grid; gap: 12px; margin: 0; padding: 0; border: 0; min-width: 0; }
  legend { padding: 0; margin-bottom: 10px; font-size: 0.72rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); }
  .hint { color: var(--text-3); font-size: 0.85rem; }
  .error { color: var(--danger); }
  .skeleton.line { height: 42px; border-radius: 10px; }
  .chips-skel { height: 32px; border-radius: 99px; width: 70%; }
  .ratings { display: flex; flex-wrap: wrap; gap: 6px; }
  .rating { height: 32px; padding: 0 13px; border-radius: 99px; border: 1px solid var(--line-strong); font-size: 0.84rem; font-weight: 600; color: var(--text-2); transition: all var(--t) var(--ease-out); }
  .rating:hover { border-color: var(--text-3); color: var(--text); }
  .rating.on { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
  .link { color: var(--accent); font-weight: 600; text-decoration: underline; text-underline-offset: 2px; }
  details summary { cursor: pointer; color: var(--text-2); font-size: 0.86rem; font-weight: 550; margin-bottom: 8px; }
  .toggle { display: flex; align-items: center; gap: 12px; cursor: pointer; margin-top: 4px; }
  .toggle span { display: grid; }
  .toggle strong { font-size: 0.9rem; font-weight: 600; }
  .toggle small { color: var(--text-3); font-size: 0.8rem; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .suffix { position: relative; display: block; }
  .suffix span { position: absolute; right: 13px; top: 50%; translate: 0 -50%; color: var(--text-3); font-size: 0.84rem; pointer-events: none; }
  .suffix input { padding-right: 52px !important; }
  @media (max-width: 520px) { .two { grid-template-columns: 1fr; } }
</style>
