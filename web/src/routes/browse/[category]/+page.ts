// SPDX-License-Identifier: AGPL-3.0-only

import { error } from "@sveltejs/kit";
import { CategoryShelvesStore } from "$houdini";
import { CATEGORIES } from "$lib/categories.js";
import { routeQuery } from "$lib/graphql.js";
import { categoryWorks, worksVariables } from "$lib/workFilters.js";

export async function load(event) {
  const category = CATEGORIES.find((item) => item.slug === event.params.category);
  if (!category) error(404, "There’s nothing to browse here.");
  if (category.shelves) return { category, shelves: await routeQuery(event, new CategoryShelvesStore(), { category: category.value as never }), works: null };
  return { category, shelves: null, works: await routeQuery(event, categoryWorks(category.slug), { category: category.value as never, ...worksVariables(event.url.searchParams) }) };
}
