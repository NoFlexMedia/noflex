// SPDX-License-Identifier: AGPL-3.0-only

import { LibraryWorksStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";
import { worksVariables } from "$lib/workFilters.js";

// One store per library, so changing a filter keeps the current grid on
// screen (dimmed) while the next one loads.
const stores = new Map<string, LibraryWorksStore>();

export async function load(event) {
  const { id } = event.params;
  let store = stores.get(id);
  if (!store) stores.set(id, (store = new LibraryWorksStore()));
  return { id, works: await routeQuery(event, store, { library: id, ...worksVariables(event.url.searchParams) }) };
}
