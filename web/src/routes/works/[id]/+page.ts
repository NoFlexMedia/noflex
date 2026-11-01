// SPDX-License-Identifier: AGPL-3.0-only

import { WorkDetailStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";

export async function load(event) {
  return { detail: await routeQuery(event, new WorkDetailStore(), { id: event.params.id }) };
}
