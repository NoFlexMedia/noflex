<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import Gear from "phosphor-svelte/lib/Gear";
  import Lock from "phosphor-svelte/lib/Lock";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import Trash from "phosphor-svelte/lib/Trash";
  import { untrack } from "svelte";
  import { cache, graphql, ParentalTagsStore } from "$houdini";
  import { date, splitList } from "$lib/format.js";
  import { fetchQuery, refetch } from "$lib/graphql.js";
  import { app, SHOW_LIBRARIES_KEY } from "$lib/session.svelte.js";
  import { attempt, confirmAction } from "$lib/ui.svelte.js";
  import Select from "$components/ui/Select.svelte";

  let { account } = $props();

  const QUALITIES = [
    { value: "auto", label: "Automatic", hint: "Best for the connection" },
    { value: "original", label: "Original", hint: "Never transcode" },
    { value: "2160", label: "4K" },
    { value: "1080", label: "1080p" },
    { value: "720", label: "720p" },
    { value: "480", label: "480p", hint: "Data saver" },
  ];

  const ratings = new ParentalTagsStore();
  $effect(() => { fetchQuery(ratings); });
  const changePasswordMutation = graphql(`
    mutation ChangePassword($current: String!, $next: String!) { changePassword(currentPassword: $current, newPassword: $next) }
  `);
  const setPreference = graphql(`
    mutation SetPreference($key: String!, $value: JSON!) {
      setPreference(key: $key, value: $value) { userId key value updatedAt ...Preferences_upsert }
    }
  `);
  const deletePreference = graphql(`
    mutation DeletePreference($key: String!) { deletePreference(key: $key) }
  `);
  let data = $derived($account.data);
  let limits = $derived(data?.myRestrictions);
  let preferences = $derived(data?.preferences ?? []);
  let ratingNames = $derived.by(() => {
    const names = new Map(($ratings.data?.tags.nodes ?? []).map((item) => [item.tag.id, item.tag.display]));
    return (limits?.allowedParentalRatingIds ?? []).map((id) => names.get(id)).filter(Boolean);
  });

  // ---- Password ----
  let password = $state({ current: "", next: "", confirm: "" });
  let changing = $state(false);
  let mismatch = $derived(password.confirm.length > 0 && password.next !== password.confirm);

  async function changePassword(event) {
    event.preventDefault();
    if (password.next !== password.confirm) return;
    changing = true;
    const ok = await attempt(() => changePasswordMutation.mutate({ current: password.current, next: password.next }), "Password changed — other devices will need to sign in again");
    changing = false;
    if (ok) { password = { current: "", next: "", confirm: "" }; refetch(account); }
  }

  // ---- Known preferences ----
  let stored = $derived(preferences.find((item) => item.key === "playback")?.value);
  let playback = $state({ quality: "auto", autoplayNext: true, subtitles: false, subtitleLanguages: "", audioLanguages: "" });
  // Re-seed the form whenever the stored value changes, including via the raw editor.
  let storedJson = $derived(JSON.stringify(stored ?? null));
  $effect(() => {
    storedJson;
    untrack(() => {
      const value = stored && typeof stored === "object" ? stored : {};
      playback = {
        quality: String(value.quality ?? "auto"),
        autoplayNext: value.autoplayNext ?? true,
        subtitles: Boolean(value.subtitles?.enabled),
        subtitleLanguages: (value.subtitles?.languages ?? []).join(", "),
        audioLanguages: (value.audio?.languages ?? []).join(", "),
      };
    });
  });
  let savingPlayback = $state(false);

  async function savePlayback(event) {
    event.preventDefault();
    const base = stored && typeof stored === "object" && !Array.isArray(stored) ? stored : {};
    // Merge so fields other clients wrote survive.
    const value = {
      ...base,
      quality: playback.quality,
      autoplayNext: playback.autoplayNext,
      subtitles: { ...(base.subtitles ?? {}), enabled: playback.subtitles, languages: splitList(playback.subtitleLanguages) },
      audio: { ...(base.audio ?? {}), languages: splitList(playback.audioLanguages) },
    };
    savingPlayback = true;
    if (await attempt(() => setPreference.mutate({ key: "playback", value }), "Playback preferences saved")) app.playbackPrefs = readPlaybackPrefs(value);
    savingPlayback = false;
  }

  // ---- Browsing ----
  async function toggleLibraries(event) {
    const input = event.currentTarget;
    const show = input.checked;
    if (await attempt(() => setPreference.mutate({ key: SHOW_LIBRARIES_KEY, value: show }))) app.showLibraries = show;
    else input.checked = app.showLibraries;
  }

  // ---- Raw preferences ----
  let raw = $state({ key: "", value: "" });
  let rawError = $state("");
  let savingRaw = $state(false);

  function editRaw(preference) {
    raw = { key: preference.key, value: JSON.stringify(preference.value, null, 2) };
    rawError = "";
  }

  async function saveRaw(event) {
    event.preventDefault();
    let value;
    try { value = JSON.parse(raw.value); } catch (cause) { rawError = `Not valid JSON: ${cause.message}`; return; }
    rawError = "";
    savingRaw = true;
    const ok = await attempt(() => setPreference.mutate({ key: raw.key.trim(), value }), `Saved “${raw.key.trim()}”`);
    savingRaw = false;
    if (ok) raw = { key: "", value: "" };
  }

  async function removeRaw(preference) {
    if (!(await confirmAction({ title: `Delete “${preference.key}”?`, message: "Clients fall back to their defaults for this setting.", confirmLabel: "Delete", danger: true }))) return;
    if (await attempt(() => deletePreference.mutate({ key: preference.key }), "Preference deleted")) cache.get("UserPreference", { userId: preference.userId, key: preference.key }).delete();
  }

  const preview = (value) => { const text = JSON.stringify(value); return text.length > 80 ? `${text.slice(0, 80)}…` : text; };
</script>

{#if ($account.fetching && !$account.data)}
  <div class="grid" aria-busy="true">
    {#each [4, 3, 5] as lines}
      <div class="panel"><div class="skeleton head"></div>{#each Array(lines) as _}<div class="skeleton line"></div>{/each}</div>
    {/each}
  </div>
{:else if data}
  <div class="grid stagger">
    <section class="panel" style:--i={0}>
      <header class="panel-head">
        <span class="panel-icon"><ShieldCheck size={19} /></span>
        <div><h2>Playback limits</h2><p>Set by your server’s administrator</p></div>
      </header>
      <dl class="facts">
        <div><dt>Content ratings</dt><dd>{#if !limits.allowedParentalRatingIds.length}Every rating{:else if ratingNames.length}{ratingNames.join(", ")}{:else}{limits.allowedParentalRatingIds.length} allowed{/if}</dd></div>
        <div><dt>Unrated titles</dt><dd>{limits.allowUnrated ? "Visible" : "Hidden"}</dd></div>
        <div><dt>Stream bitrate</dt><dd>{limits.maxStreamBitrateBps ? `Up to ${Math.round(limits.maxStreamBitrateBps / 100_000) / 10} Mbps` : "No cap"}</dd></div>
        <div><dt>Simultaneous transcodes</dt><dd>{limits.maxConcurrentTranscodes ?? "Server default"}</dd></div>
      </dl>
    </section>

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Lock size={19} /></span>
        <div><h2>Password</h2><p>Changing it signs out every other device</p></div>
      </header>
      <form class="stack" onsubmit={changePassword}>
        <label class="field">Current password<input type="password" bind:value={password.current} autocomplete="current-password" required /></label>
        <label class="field">New password<input type="password" bind:value={password.next} autocomplete="new-password" minlength="8" required /><small>At least 8 characters.</small></label>
        <label class="field">Confirm new password<input type="password" bind:value={password.confirm} autocomplete="new-password" required aria-invalid={mismatch} aria-describedby="pw-mismatch" />{#if mismatch}<small id="pw-mismatch" class="error">Passwords don’t match yet.</small>{/if}</label>
        <div class="form-actions"><button class="btn primary sm" disabled={changing || mismatch || !password.current || password.next.length < 8}>{#if changing}<span class="spinner sm"></span>{/if}Update password</button></div>
      </form>
    </section>

    <section class="panel wide" style:--i={2}>
      <header class="panel-head">
        <span class="panel-icon"><Books size={19} /></span>
        <div><h2>Browsing</h2><p>NoFlex browses by movies, shows and other content types across your libraries</p></div>
      </header>
      <div class="toggles">
        <label class="toggle"><input type="checkbox" class="switch" checked={app.showLibraries} onchange={toggleLibraries} /><span><strong>Show libraries in the sidebar</strong><small>List each library too, for browsing by where files live.</small></span></label>
      </div>
    </section>

    <section class="panel wide" style:--i={2}>
      <header class="panel-head">
        <span class="panel-icon"><Gear size={19} /></span>
        <div><h2>Playback preferences</h2><p>Synced to your account, so every NoFlex client can honour them</p></div>
      </header>
      <form class="stack" onsubmit={savePlayback}>
        <div class="form-grid">
          <label class="field" for="pref-quality">Streaming quality<Select id="pref-quality" bind:value={playback.quality} items={QUALITIES} label="Streaming quality" /></label>
          <label class="field">Preferred audio languages<input bind:value={playback.audioLanguages} placeholder="en, ja" /><small>Comma-separated, most preferred first.</small></label>
          <label class="field">Preferred subtitle languages<input bind:value={playback.subtitleLanguages} placeholder="en" /><small>Used when subtitles are on.</small></label>
        </div>
        <div class="toggles">
          <label class="toggle"><input type="checkbox" class="switch" bind:checked={playback.autoplayNext} /><span><strong>Autoplay next episode</strong><small>Roll straight into the next episode or track.</small></span></label>
          <label class="toggle"><input type="checkbox" class="switch" bind:checked={playback.subtitles} /><span><strong>Subtitles on by default</strong><small>Pick a matching track when one exists.</small></span></label>
        </div>
        <div class="form-actions"><button class="btn primary sm" disabled={savingPlayback}>{#if savingPlayback}<span class="spinner sm"></span>{/if}Save preferences</button></div>
      </form>

      <details class="advanced">
        <summary><CaretRight size={15} />Advanced <span class="faint">· {preferences.length} stored {preferences.length === 1 ? "key" : "keys"}</span></summary>
        <div class="advanced-body">
          {#if preferences.length}
            <ul class="list prefs">
              {#each preferences as preference (preference.key)}
                <li>
                  <span class="pref"><code>{preference.key}</code><small class="mono">{preview(preference.value)}</small></span>
                  <small class="when">{date(preference.updatedAt)}</small>
                  <button class="icon-btn sm" aria-label="Edit {preference.key}" onclick={() => editRaw(preference)}><PencilSimple size={15} /></button>
                  <button class="icon-btn sm" aria-label="Delete {preference.key}" onclick={() => removeRaw(preference)}><Trash size={15} /></button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="empty-note">No preferences stored yet.</p>
          {/if}
          <form class="raw" onsubmit={saveRaw}>
            <label class="field">Key<input bind:value={raw.key} required placeholder="player.theme" spellcheck="false" autocomplete="off" /></label>
            <label class="field">JSON value<textarea class="mono" bind:value={raw.value} required rows="4" placeholder={'true, "dark" or { "any": "object" }'} spellcheck="false" aria-invalid={!!rawError} aria-describedby="raw-error"></textarea>{#if rawError}<small id="raw-error" class="error">{rawError}</small>{/if}</label>
            <div class="form-actions">
              {#if raw.key || raw.value}<button type="button" class="btn ghost sm" onclick={() => { raw = { key: "", value: "" }; rawError = ""; }}>Clear</button>{/if}
              <button class="btn sm" disabled={savingRaw || !raw.key.trim()}>{#if savingRaw}<span class="spinner sm"></span>{/if}Save key</button>
            </div>
          </form>
        </div>
      </details>
    </section>
  </div>
{/if}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 360px), 1fr)); gap: 18px; align-items: start; }
  .wide { grid-column: 1 / -1; }
  .skeleton.head { height: 38px; width: 60%; margin-bottom: 22px; border-radius: 11px; }
  .skeleton.line { height: 16px; margin-top: 14px; border-radius: 6px; }
  .skeleton.line:nth-child(odd) { width: 72%; }
  .facts { display: grid; grid-template-columns: 1fr 1fr; gap: 18px 24px; }
  dt { font-size: 0.74rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 4px; }
  dd { font-size: 0.96rem; font-weight: 550; }
  .error { color: var(--danger) !important; }
  .toggles { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr)); gap: 10px; }
  .toggle { display: flex; align-items: flex-start; gap: 14px; padding: 14px; border-radius: var(--r); background: var(--surface); border: 1px solid var(--line); cursor: pointer; transition: border-color var(--t); }
  .toggle:hover { border-color: var(--line-strong); }
  .toggle span { display: grid; gap: 2px; }
  .toggle strong { font-size: 0.92rem; font-weight: 600; }
  .toggle small { color: var(--text-3); font-size: 0.82rem; }
  .advanced { margin-top: 22px; border-top: 1px solid var(--line); padding-top: 14px; }
  .advanced summary { display: inline-flex; align-items: center; gap: 8px; cursor: pointer; font-weight: 600; font-size: 0.9rem; color: var(--text-2); list-style: none; border-radius: 8px; }
  .advanced summary::-webkit-details-marker { display: none; }
  .advanced summary :global(svg) { transition: transform var(--t) var(--ease-out); }
  .advanced[open] summary :global(svg) { transform: rotate(90deg); }
  .advanced summary:hover { color: var(--text); }
  .advanced-body { display: grid; gap: 18px; margin-top: 14px; animation: fade-up 300ms var(--ease-out); }
  .prefs li { gap: 10px; padding-block: 9px; }
  .pref { flex: 1; display: grid; min-width: 0; gap: 2px; }
  .pref code { font-size: 0.86rem; color: var(--text); }
  .pref small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { white-space: nowrap; }
  .raw { display: grid; grid-template-columns: minmax(160px, 1fr) 2fr; gap: 12px; align-items: start; }
  .raw .form-actions { grid-column: 1 / -1; }
  textarea.mono { font-size: 0.84rem; }
  @media (max-width: 620px) { .raw, .facts { grid-template-columns: 1fr; } .when { display: none; } }
</style>
