// SPDX-License-Identifier: AGPL-3.0-only

import { HomeStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";

export async function load(event) {
  return { home: await routeQuery(event, new HomeStore()) };
}
