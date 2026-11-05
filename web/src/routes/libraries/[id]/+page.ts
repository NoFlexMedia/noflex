// SPDX-License-Identifier: AGPL-3.0-only

import { LibraryWorksStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";
import { activateServer, connectedServers, deactivateServer, ensureLinkedServers, loadServerLibraries, servers } from "$lib/servers.svelte.js";
import { hasSession } from "$lib/tokens.js";
import { worksVariables } from "$lib/workFilters.js";

// One store per library, so changing a filter keeps the current grid on
// screen (dimmed) while the next one loads.
const stores = new Map<string, LibraryWorksStore>();

export async function load(event) {
  const { id } = event.params;
  // Links within a linked server's library carry its id, so every page of it says which server it's on.
  const serverId = event.url.searchParams.get("server");
  if (serverId && hasSession()) {
    await ensureLinkedServers();
    activateServer(serverId);
    if (!servers.libraries[serverId]) await loadServerLibraries(connectedServers().filter((server) => server.id === serverId));
  } else {
    deactivateServer();
  }
  let store = stores.get(id);
  if (!store) stores.set(id, (store = new LibraryWorksStore()));
  return { id, serverId, works: await routeQuery(event, store, { library: id, ...worksVariables(event.url.searchParams) }) };
}
