<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { DropdownMenu } from "bits-ui";
  import Check from "phosphor-svelte/lib/Check";
  import MonitorPlay from "phosphor-svelte/lib/MonitorPlay";
  import { connect } from "$lib/connect.svelte.js";
  import DeviceIcon from "$components/DeviceIcon.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import Tip from "$components/ui/Tip.svelte";
</script>

<!-- Offered only while there's somewhere to play, as a cast button is. -->
{#if connect.available.length > 0 || connect.target}
  <Menu width={260}>
    {#snippet trigger(props)}
      <Tip text={connect.target ? `Playing on ${connect.target.device.name}` : "Play on a device"}>
        {#snippet children(tip)}
          <button {...props} {...tip} type="button" class="icon-btn sm" class:on={connect.target} aria-pressed={!!connect.target} aria-label="Play on a device"><MonitorPlay size={19} weight={connect.target ? "fill" : "regular"} /></button>
        {/snippet}
      </Tip>
    {/snippet}
    <div class="nf-menu-label">Play on</div>
    <DropdownMenu.Item class="nf-menu-item" onSelect={() => connect.setTarget(null)}>
      <DeviceIcon device={null} client="NoFlex Web" size={17} /><span>This browser</span>{#if !connect.target}<Check size={15} />{/if}
    </DropdownMenu.Item>
    {#each connect.available as item (item.device.id)}
      <DropdownMenu.Item class="nf-menu-item" onSelect={() => connect.setTarget(item.device.id)}>
        <DeviceIcon device={item.device.name} client={item.device.client} size={17} /><span>{item.device.name}</span>{#if connect.targetId === item.device.id}<Check size={15} />{/if}
      </DropdownMenu.Item>
    {/each}
  </Menu>
{/if}

<style>
  .icon-btn.on { color: var(--accent); }
</style>
