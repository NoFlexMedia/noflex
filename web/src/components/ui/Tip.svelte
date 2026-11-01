<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { Tooltip } from "bits-ui";

  /** `children` receives trigger props to spread onto the target element. */
  let { text, side = "top", hint = null, contentClass = "", portal = undefined, children } = $props();
</script>

<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}{@render children(props)}{/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal to={portal}>
    <Tooltip.Content class="nf-tip {contentClass}" {side} sideOffset={8}>{text}{#if hint}<kbd>{hint}</kbd>{/if}</Tooltip.Content>
  </Tooltip.Portal>
</Tooltip.Root>
