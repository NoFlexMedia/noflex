// SPDX-License-Identifier: AGPL-3.0-only

import { WatchRoomsStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";

export async function load(event) {
  return { rooms: await routeQuery(event, new WatchRoomsStore()) };
}
