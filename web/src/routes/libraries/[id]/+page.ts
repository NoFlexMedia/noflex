// SPDX-License-Identifier: AGPL-3.0-only

import { LibraryWorksStore } from "$houdini";
import { accessClaims } from "$lib/api.js";
import { routeQuery } from "$lib/graphql.js";

// One store per library, so changing a filter keeps the current grid on
// screen (dimmed) while the next one loads.
const stores = new Map<string, LibraryWorksStore>();

function variables(library: string, params: URLSearchParams) {
  // Loads can run before the viewer is known; the token names them.
  const viewer = accessClaims().sub;
  const filter: Record<string, unknown> = { recursive: params.get("all") === "1" };
  const kind = params.get("kind");
  if (kind) { filter.kinds = [kind]; filter.recursive = true; }
  if (params.get("from")) filter.yearMin = Number(params.get("from"));
  if (params.get("to")) filter.yearMax = Number(params.get("to"));
  if (params.get("rating")) filter.ratingMin = Number(params.get("rating"));
  if (params.get("favorites") === "1") filter.favoriteFor = viewer;
  if (params.get("watched")) filter.watched = { userId: viewer, watched: params.get("watched") === "watched" };
  if (params.get("tag")) filter.tag = params.get("tag");
  return { library, filter, sort: (params.get("sort") ?? "DATE_ADDED_DESC") as never };
}

export async function load(event) {
  const { id } = event.params;
  let store = stores.get(id);
  if (!store) stores.set(id, (store = new LibraryWorksStore()));
  return { id, works: await routeQuery(event, store, variables(id, event.url.searchParams)) };
}
