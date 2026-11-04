// SPDX-License-Identifier: AGPL-3.0-only
//
// Grid filters live in the URL, so loaders and the grid read one source.

import { accessClaims } from "$lib/api.js";

/** `works` arguments for a grid's URL parameters. */
export function worksVariables(params: URLSearchParams) {
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
  return { filter, sort: (params.get("sort") ?? "DATE_ADDED_DESC") as never };
}
