// SPDX-License-Identifier: AGPL-3.0-only

import { error } from "@sveltejs/kit";
import { CategoryWorksStore } from "$houdini";
import { CATEGORIES } from "$lib/categories.js";
import { routeQuery } from "$lib/graphql.js";
import { worksVariables } from "$lib/workFilters.js";

// One store per category, so changing a filter keeps the current grid on
// screen (dimmed) while the next one loads.
const stores = new Map<string, CategoryWorksStore>();

export async function load(event) {
  const category = CATEGORIES.find((item) => item.slug === event.params.category);
  if (!category) error(404, "There’s nothing to browse here.");
  let store = stores.get(category.slug);
  if (!store) stores.set(category.slug, (store = new CategoryWorksStore()));
  return { category, works: await routeQuery(event, store, { category: category.value as never, ...worksVariables(event.url.searchParams) }) };
}
