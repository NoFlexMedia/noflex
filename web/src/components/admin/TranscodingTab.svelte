<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import Check from "phosphor-svelte/lib/Check";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import Monitor from "phosphor-svelte/lib/Monitor";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import Stack from "phosphor-svelte/lib/Stack";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import X from "phosphor-svelte/lib/X";
  import { AdminTranscoderStore, graphql } from "$houdini";
  import { clientCapabilities } from "$lib/codecSupport.js";
  import { bytes, pixelTier, profileName } from "$lib/format.js";
  import { failure, fetchQuery, refetch } from "$lib/graphql.js";
  import { attempt, poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Select from "$components/ui/Select.svelte";
  import { ago } from "$components/account/relative.js";
  import { codec } from "./status/fmt.js";

  const transcoder = new AdminTranscoderStore();
  $effect(() => { fetchQuery(transcoder); });

  const versions = graphql(`
    query AdminOptimizedVersions {
      optimizedVersions { id variantKey status progress sizeBytes error description lastUsedAt finishedAt work { id title } }
    }
  `);
  $effect(() => { fetchQuery(versions); });
  $effect(() => poll(() => refetch(versions), 5000));
  let versionList = $derived($versions.data?.optimizedVersions ?? []);
  let versionCount = (status) => versionList.filter((version) => version.status === status).length;
  let preparedGb = $derived(versionList.reduce((total, version) => total + (version.status === "READY" ? version.sizeBytes ?? 0 : 0), 0) / 1024 ** 3);
  // What needs a look first, then the newest of what's done.
  const STATUS_ORDER = { RUNNING: 0, FAILED: 1, QUEUED: 2, READY: 3 };
  const STATUS_LABEL = { RUNNING: "Preparing", FAILED: "Failed", QUEUED: "In the queue", READY: "Ready" };
  let sortedVersions = $derived([...versionList].sort((a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || String(b.finishedAt ?? "").localeCompare(String(a.finishedAt ?? ""))));

  const smart = graphql(`
    query AdminSmartTranscoding {
      smartTranscoding { enabled directory maxStorageGb maxVersionGb defaultDirectory defaultMaxStorageGb defaultMaxVersionGb }
    }
  `);
  const saveSmart = graphql(`
    mutation UpdateSmartTranscoding($directory: String, $storage: Int, $version: Int) {
      updateSmartTranscoding(directory: $directory, maxStorageGb: $storage, maxVersionGb: $version) { enabled directory maxStorageGb maxVersionGb defaultDirectory defaultMaxStorageGb defaultMaxVersionGb }
    }
  `);
  $effect(() => { fetchQuery(smart); });
  let settings = $derived($smart.data?.smartTranscoding);
  // Blank fields fall back to the server's configuration.
  let draft = $state({ directory: "", storage: "", version: "" });
  let saving = $state(false);
  $effect(() => {
    if (!settings) return;
    draft = {
      directory: settings.directory === settings.defaultDirectory ? "" : settings.directory,
      storage: settings.maxStorageGb === settings.defaultMaxStorageGb ? "" : String(settings.maxStorageGb),
      version: settings.maxVersionGb == null || settings.maxVersionGb === settings.defaultMaxVersionGb ? "" : String(settings.maxVersionGb),
    };
  });
  const toGb = (text) => (String(text).trim() === "" ? null : Number(text));
  async function saveSettings(event) {
    event.preventDefault();
    saving = true;
    if (await attempt(() => saveSmart.mutate({ directory: draft.directory.trim() || null, storage: toGb(draft.storage), version: toGb(draft.version) }), "Smart transcoding settings saved")) refetch(smart);
    saving = false;
  }

  const curve = graphql(`
    query AdminTonemapCurve { tonemapCurve }
  `);
  const saveCurve = graphql(`
    mutation SetTonemapCurve($curve: TonemapCurve) { setTonemapCurve(curve: $curve) }
  `);
  $effect(() => { fetchQuery(curve); });
  const CURVES = [
    { value: "HABLE", label: "Hable", hint: "Filmic, soft highlights" },
    { value: "MOBIUS", label: "Möbius", hint: "Keeps midtones, compresses highlights" },
    { value: "REINHARD", label: "Reinhard", hint: "Simple and even" },
    { value: "SPLINE", label: "Spline", hint: "Bright midtones", gpu: true },
    { value: "BT2390", label: "BT.2390", hint: "The broadcast standard; brightest", gpu: true },
  ];
  const chooseCurve = async (value) => { if (await attempt(() => saveCurve.mutate({ curve: value }), "Tone-mapping curve saved")) refetch(curve); };

  let info = $derived($transcoder.data?.transcoderInfo);
  let onlyAvailable = $state(false);
  let showFlags = $state({ ffmpeg: false, ffprobe: false });

  let browser = $state(null);
  $effect(() => { clientCapabilities().then((caps) => (browser = caps), () => {}); });
  const megapixels = (pixels) => `${(pixels / 1e6).toFixed(pixels < 1e7 ? 1 : 0)} MP`;
  /** Whether this browser could play a rung's encode, or null before it's measured. */
  function browserPlays(rung) {
    if (!browser) return null;
    const support = browser.videoCodecs.find((item) => item.codec === rung.videoCodec);
    return !!support && browser.hls.some((hls) => hls.videoCodecs.includes(rung.videoCodec)) && support.profiles.includes(rung.videoProfile) && (support.maxPixels == null || support.maxPixels >= rung.maxPixels);
  }

  const BACKEND = { software: "Software", vaapi: "VAAPI", nvenc: "NVENC", qsv: "Quick Sync", videotoolbox: "VideoToolbox", mf: "Media Foundation", cuda: "CUDA (NVDEC)" };
  const backendLabel = (name) => BACKEND[name] ?? name;

  function grouped(list) {
    const groups = new Map();
    for (const item of list) {
      if (onlyAvailable && !item.available) continue;
      const key = `${item.media}:${item.backend}`;
      if (!groups.has(key)) groups.set(key, { media: item.media, backend: item.backend, items: [] });
      groups.get(key).items.push(item);
    }
    return [...groups.values()];
  }
  let encoderGroups = $derived(info ? grouped(info.encoders) : []);
  let decoderGroups = $derived(info ? grouped(info.decoders) : []);
  let binaries = $derived(info ? [["ffmpeg", info.ffmpeg], ["ffprobe", info.ffprobe]] : []);
</script>

{#if $transcoder.fetching && !info}
  <div class="diag"><div class="diag-cols">{#each Array(2) as _}<div class="panel skeleton tall"></div>{/each}</div><div class="panel skeleton tall"></div></div>
{:else if failure($transcoder) && !info}
  <State kind="error" title="Couldn’t read the transcoder’s state" message={failure($transcoder)} action={() => refetch(transcoder)} />
{:else if info}
  <div class="diag">
    <div class="diag-cols">
      {#each binaries as [name, bin] (name)}
        <section class="panel dense">
          <header class="panel-head">
            <span class="panel-icon"><FilmStrip size={18} /></span>
            <div><h2>{name}</h2><p>{bin.version?.replace(/\s+Copyright.*$/, "") ?? "Version unknown"}</p></div>
            <span class="spacer"></span>
            <span class="state" class:ok={!bin.error} class:bad={bin.error}>{bin.error ? "Unavailable" : "Found"}</span>
          </header>
          {#if bin.error}<p class="error"><WarningCircle size={14} />{bin.error}</p>{/if}
          <dl class="kv wide">
            <div class="full"><dt>Resolved path</dt><dd class="mono">{bin.path ?? "—"}</dd></div>
            <div><dt>Invoked as</dt><dd class="mono">{bin.command}</dd></div>
            <div><dt>Override with</dt><dd class="mono">{bin.overrideEnv}</dd></div>
          </dl>
          {#if bin.buildFlags.length}
            <button class="btn ghost sm flags-toggle" aria-expanded={showFlags[name]} onclick={() => (showFlags[name] = !showFlags[name])}>
              <CaretDown size={14} />{bin.buildFlags.length} build flags
            </button>
            {#if showFlags[name]}<div class="flags">{#each bin.buildFlags as flag (flag)}<code>{flag.replace("--enable-", "")}</code>{/each}</div>{/if}
          {/if}
        </section>
      {/each}
    </div>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Stack size={18} /></span>
        <div><h2>Smart transcoding</h2><p>{versionCount("READY")} ready · {preparedGb.toFixed(1)}{settings ? ` of ${settings.maxStorageGb}` : ""} GB · {versionCount("QUEUED")} queued</p></div>
      </header>
      {#if settings && !settings.enabled}
        <p class="error warn"><WarningCircle size={14} />Turned off by <code>transcoder.optimize.enabled</code> in the server’s configuration.</p>
      {/if}
      {#if settings}
        <form class="smart-form" onsubmit={saveSettings}>
          <label class="field wide">Folder<input class="mono" bind:value={draft.directory} placeholder={settings.defaultDirectory} spellcheck="false" autocomplete="off" /><small>Versions take real disk, so avoid cache volumes. Moving this leaves existing versions where they are, still playing.</small></label>
          <label class="field">Total storage (GB)<input type="number" min="0" step="1" inputmode="numeric" bind:value={draft.storage} placeholder={String(settings.defaultMaxStorageGb)} /><small>Past this, the least recently used are removed.</small></label>
          <label class="field">Largest version (GB)<input type="number" min="1" step="1" inputmode="numeric" bind:value={draft.version} placeholder={settings.defaultMaxVersionGb == null ? "No limit" : String(settings.defaultMaxVersionGb)} /><small>Titles estimated bigger than this play with a live transcode instead.</small></label>
          <div class="smart-actions"><button class="btn sm primary" disabled={saving}>{#if saving}<span class="spinner sm"></span>{/if}Save</button></div>
        </form>
      {/if}
      {#if sortedVersions.length}
        <ul class="versions">
          {#each sortedVersions as version (version.id)}
            <li>
              <div class="version-main">
                {#if version.work}<a href="/works/{version.work.id}">{version.work.title}</a>{:else}<span>A removed title</span>{/if}
                <small title={version.variantKey}>{version.description}</small>
                {#if version.status === "FAILED"}<small class="failed"><WarningCircle size={14} />{version.error ?? "Failed"}</small>{/if}
              </div>
              <div class="version-side">
                <span class="state" class:accent={version.status === "RUNNING"}>{version.status === "RUNNING" ? `${STATUS_LABEL.RUNNING} ${Math.round(version.progress * 100)}%` : STATUS_LABEL[version.status]}</span>
                {#if version.status === "READY"}<small class="tabular">{bytes(version.sizeBytes)} · {version.lastUsedAt ? `last played ${ago(version.lastUsedAt)}` : "not played yet"}</small>{/if}
              </div>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted-note">Nothing prepared yet. Versions appear here once viewers mark titles or turn smart transcoding on for a device.</p>
      {/if}
    </section>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Sparkle size={18} /></span>
        <div><h2>Hardware acceleration</h2><p>Resolved once at startup from <code>transcoder.hwaccel</code> and what ffmpeg reports.</p></div>
        <span class="spacer"></span>
        <span class="state" class:accent={info.hardwareAccelerated}>{info.hardwareAccelerated ? "Hardware" : "Software only"}</span>
      </header>
      {#if info.fellBack}
        <p class="error warn"><WarningCircle size={14} />The configured <strong>{info.configuredHwaccel}</strong> backend wasn’t available, so the <strong>{info.fallback}</strong> fallback picked {backendLabel(info.resolvedBackend)}.</p>
      {/if}
      <dl class="kv">
        <div><dt>In use</dt><dd>{backendLabel(info.resolvedBackend)}</dd></div>
        <div><dt>Configured</dt><dd class="mono">{info.configuredHwaccel}</dd></div>
        <div><dt>Fallback</dt><dd class="mono">{info.fallback}</dd></div>
        <div><dt>VAAPI device</dt><dd class="mono">{info.vaapiDevice ?? "none"}</dd></div>
        <div><dt>HDR tone-mapping</dt><dd>{info.gpuTonemap ? "GPU (Vulkan)" : "CPU"}</dd></div>
        {#if $curve.data}
          <div class="curve">
            <dt>Tone-mapping curve</dt>
            <dd><Select value={$curve.data.tonemapCurve} items={CURVES.map((item) => ({ ...item, disabled: item.gpu && !info.gpuTonemap, hint: item.gpu && !info.gpuTonemap ? "Needs GPU tone-mapping" : item.hint }))} label="Tone-mapping curve" onchange={chooseCurve} /></dd>
          </div>
        {/if}
        <div class="full"><dt>ffmpeg hwaccels</dt><dd class="chips">{#each info.hwaccels as accel (accel)}<span class="chip">{accel}</span>{:else}<span class="faint">None</span>{/each}</dd></div>
        <div class="full"><dt>Transcode cache</dt><dd class="mono">{info.transcodeDir}</dd></div>
      </dl>
    </section>

    {#if browser}
      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><Monitor size={18} /></span>
          <div><h2>This browser</h2><p>What it reported it can decode. Sizes are each profile’s largest picture, bounded by its level.</p></div>
        </header>
        <div class="dtable-wrap">
          <table class="dtable">
            <thead><tr><th scope="col">Video</th><th scope="col">Best profile</th><th scope="col">Max picture</th><th scope="col">Bit depth</th><th scope="col">Dynamic range</th></tr></thead>
            <tbody>
              {#each browser.videoCodecs as support (support.codec)}
                <tr>
                  <td class="strong">{codec(support.codec)}</td>
                  <td>{profileName(support.profiles[0]) || "—"}</td>
                  <td class="nowrap" title={support.maxPixels ? megapixels(support.maxPixels) : null}>{support.maxPixels ? pixelTier(support.maxPixels) : "—"}</td>
                  <td>{support.maxBitDepth ? `${support.maxBitDepth}-bit` : "—"}</td>
                  <td>{support.videoRanges.join(", ").replaceAll("_", " ")}</td>
                </tr>
              {:else}
                <tr><td colspan="5" class="faint">No video codecs reported.</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/if}

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Stack size={18} /></span>
        <div><h2>Transcode ladder</h2><p>A re-encode uses the smallest rung of its codec that holds the picture. Live playback prefers what the hardware encoder can do, then H.264.</p></div>
      </header>
      <div class="dtable-wrap">
        <table class="dtable">
          <thead><tr><th scope="col">Max picture</th><th scope="col">Video profile</th><th scope="col">Audio</th><th scope="col">Container</th><th scope="col" class="num">Bitrate</th><th scope="col">Encoder</th><th scope="col">This browser</th></tr></thead>
          <tbody>
            {#each info.ladder as rung (rung.maxPixels + rung.videoCodec)}
              <tr>
                <td class="strong nowrap" title={megapixels(rung.maxPixels)}>{pixelTier(rung.maxPixels)}</td>
                <td class="nowrap">{codec(rung.videoCodec)} {profileName(rung.videoProfile)}</td>
                <td>{rung.audioCodec}</td>
                <td class="mono">{rung.container}</td>
                <td class="num">{(rung.bitrateKbps / 1000).toFixed(1)} Mb/s</td>
                <td class="nowrap"><code>{rung.encoder}</code> {#if rung.encoderAvailable}<span class="state ok">available</span>{:else}<span class="state bad">missing</span>{/if}</td>
                <td>{#if browserPlays(rung) === null}<span class="faint">—</span>{:else if browserPlays(rung)}<span class="state ok">plays</span>{:else}<span class="faint">can’t decode</span>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>

    <div class="codec-bar">
      <h2>Codecs in this ffmpeg</h2>
      <span class="spacer"></span>
      <label class="check"><input type="checkbox" bind:checked={onlyAvailable} />Available only</label>
    </div>
    <div class="diag-cols">
      {#each [["Encoders", encoderGroups], ["Decoders", decoderGroups]] as [title, groups] (title)}
        <section class="panel dense">
          <header class="panel-head"><div><h3>{title}</h3><p>{title === "Encoders" ? "Everything the ladder can map to." : "Common source formats, including GPU decoders."}</p></div></header>
          {#each groups as group (group.media + group.backend)}
            <div class="group">
              <p class="eyebrow">{group.media === "VIDEO" ? "Video" : "Audio"} · {backendLabel(group.backend)}</p>
              <div class="codecs">
                {#each group.items as item (item.name)}
                  <span class="codec" class:on={item.available} title={item.available ? "Available" : "Not in this build"}>{#if item.available}<Check size={12} />{:else}<X size={12} />{/if}{item.name}</span>
                {/each}
              </div>
            </div>
          {:else}
            <p class="empty-note">Nothing available.</p>
          {/each}
        </section>
      {/each}
    </div>
  </div>
{/if}

<style>
  .curve { min-width: 200px; }
  .smart-form { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 240px), 1fr)); gap: 14px 20px; align-items: start; margin-bottom: 14px; }
  .smart-form .wide { grid-column: 1 / -1; }
  .smart-actions { grid-column: 1 / -1; }
  .versions { display: grid; gap: 2px; max-height: 440px; overflow-y: auto; list-style: none; margin: 0; padding: 0; font-size: 0.88rem; }
  .versions li { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 6px 14px; padding: 9px 0; border-top: 1px solid var(--line); }
  .versions li:first-child { border-top: 0; }
  .version-main { display: grid; gap: 2px; min-width: 0; }
  .version-main a { font-weight: 600; }
  .version-main a:hover { text-decoration: underline; text-underline-offset: 3px; }
  .version-side { display: grid; justify-items: end; gap: 2px; }
  .versions small { color: var(--text-3); font-size: 0.8rem; }
  .versions .failed { display: inline-flex; align-items: center; gap: 6px; color: var(--danger); }
  .muted-note { color: var(--text-3); font-size: 0.86rem; }
  .tall { height: 220px; }
  .error { display: flex; align-items: flex-start; gap: 8px; color: var(--danger); font-size: 0.86rem; margin-bottom: 14px; }
  .error.warn { color: var(--warning); }
  .error :global(svg) { margin-top: 3px; flex: none; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .flags-toggle { margin-top: 12px; margin-left: -10px; }
  .flags { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; max-height: 220px; overflow-y: auto; }
  .flags code { padding: 2px 7px; border-radius: 6px; background: rgb(0 0 0 / 0.25); color: var(--text-2); font-size: 0.74rem; }
  .codec-bar { display: flex; align-items: center; gap: 12px; margin-top: 6px; }
  .codec-bar h2 { font-size: 1.04rem; }
  .group + .group { margin-top: 14px; }
  .group .eyebrow { margin-bottom: 7px; }
  .codecs { display: flex; flex-wrap: wrap; gap: 6px; }
  .codec { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 9px; border-radius: 7px; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.76rem; background: var(--surface); border: 1px solid var(--line); color: var(--text-3); text-decoration: line-through; text-decoration-color: rgb(255 255 255 / 0.2); }
  .codec.on { color: var(--text); background: rgb(110 231 168 / 0.07); border-color: rgb(110 231 168 / 0.2); text-decoration: none; }
  .codec.on :global(svg) { color: var(--success); }
</style>
