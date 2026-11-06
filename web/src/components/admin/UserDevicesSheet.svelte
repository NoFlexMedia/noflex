<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { graphql } from "$houdini";
  import { request } from "$lib/graphql.js";
  import { attempt } from "$lib/ui.svelte.js";
  import { ago } from "$components/account/relative.js";
  import DeviceIcon from "$components/DeviceIcon.svelte";
  import Dialog from "$components/ui/Dialog.svelte";

  let { open = $bindable(false), user = null } = $props();

  const devices = graphql(`
    query UserDevices($user: ID!) @cache(policy: NetworkOnly) {
      userDevices(userId: $user) { id name client smartTranscode smartDownloads lastSeenAt }
    }
  `);
  const setSmart = graphql(`
    mutation SetViewerDeviceSmartTranscode($id: ID!, $enabled: Boolean!) {
      setViewerDeviceSmartTranscode(id: $id, enabled: $enabled) { id smartTranscode smartDownloads }
    }
  `);

  let list = $state(null);
  let error = $state("");
  let userId = $derived(user?.id);
  $effect(() => {
    if (!open || !userId) return;
    const id = userId;
    list = null; error = "";
    request(devices, { user: id })
      .then((data) => { if (userId === id) list = data.userDevices; })
      .catch((cause) => { error = cause.message; });
  });

  const wants = (device) => device.smartTranscode || device.smartDownloads;

  async function toggle(device, event) {
    const input = event.currentTarget;
    const enabled = input.checked;
    const data = await attempt(() => request(setSmart, { id: device.id, enabled }));
    if (data) Object.assign(device, data.setViewerDeviceSmartTranscode);
    input.checked = wants(device);
  }
</script>

<Dialog bind:open title="{user?.name ? `${user.name}’s` : 'Their'} devices" description="Turning smart transcoding off stops the server preparing what it predicts this device will watch. They can turn it back on, and anything they’ve marked for the device stays marked." width={560}>
  {#if error}
    <p class="problem">{error}</p>
  {:else if !list}
    <span class="spinner sm"></span>
  {:else if !list.length}
    <p class="none">No devices have registered with this account yet.</p>
  {:else}
    <ul>
      {#each list as device (device.id)}
        <li>
          <span class="icon"><DeviceIcon device={device.name} client={device.client} size={20} /></span>
          <span class="what"><strong>{device.name}</strong><small>Last seen {ago(device.lastSeenAt)}</small></span>
          <label class="toggle">
            <small>Smart transcoding</small>
            <input type="checkbox" class="switch" checked={wants(device)} onchange={(event) => toggle(device, event)} aria-label="Smart transcoding on {device.name}" />
          </label>
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (open = false)}>Done</button>
  {/snippet}
</Dialog>

<style>
  ul { display: grid; gap: 2px; list-style: none; margin: 0; padding: 0; }
  li { display: flex; align-items: center; gap: 12px; padding: 10px 0; border-top: 1px solid var(--line); }
  li:first-child { border-top: 0; }
  .icon { display: grid; place-items: center; width: 36px; height: 36px; border-radius: var(--r); background: var(--surface); color: var(--text-2); flex: none; }
  .what { display: grid; gap: 2px; flex: 1; min-width: 0; font-size: 0.9rem; }
  .what strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  small { color: var(--text-3); font-size: 0.8rem; }
  .toggle { display: flex; align-items: center; gap: 10px; cursor: pointer; }
  .none { color: var(--text-2); font-size: 0.9rem; }
  .problem { color: var(--danger); font-size: 0.9rem; }
</style>
