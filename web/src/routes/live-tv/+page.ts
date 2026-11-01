// SPDX-License-Identifier: AGPL-3.0-only

import { LiveTvStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";

export async function load(event) {
  return { live: await routeQuery(event, new LiveTvStore()) };
}
