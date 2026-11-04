// SPDX-License-Identifier: AGPL-3.0-only

import { WorkDetailStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";
import { activateServer, ensureLinkedServers } from "$lib/servers.svelte.js";
import { hasSession } from "$lib/tokens.js";

export async function load(event) {
  // Links within a linked server's title stay on it without saying so again;
  // the layout returns home on leaving.
  const server = event.url.searchParams.get("server");
  if (server && hasSession()) {
    await ensureLinkedServers();
    activateServer(server);
  }
  return { detail: await routeQuery(event, new WorkDetailStore(), { id: event.params.id }) };
}
