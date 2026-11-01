<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import { page } from "$app/state";
  import State from "$components/State.svelte";
</script>

<svelte:head><title>{page.status === 404 ? "Not found" : "Something went wrong"} · NoFlex</title></svelte:head>

<div class="page">
  {#if page.status === 404}
    <State icon={MagnifyingGlass} title="Nothing lives here" message="That address doesn’t match anything on this server."><a class="btn sm" href="/">Go home</a></State>
  {:else}
    <State kind="error" title="Something went wrong" message={page.error?.message} action={() => location.reload()} actionLabel="Reload" />
  {/if}
</div>
