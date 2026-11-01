<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { DropdownMenu } from "bits-ui";

  /** `trigger` receives bits' trigger props to spread onto your own button. */
  let { trigger, children, align = "end", side = "bottom", width = 220, portal = undefined } = $props();
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}{@render trigger(props)}{/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal to={portal}>
    <DropdownMenu.Content class="nf-menu" {align} {side} sideOffset={8} collisionPadding={12} style="--menu-w: {width}px">
      {@render children()}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
