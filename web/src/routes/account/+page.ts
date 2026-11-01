// SPDX-License-Identifier: AGPL-3.0-only

import { AccountStore } from "$houdini";
import { routeQuery } from "$lib/graphql.js";

export async function load(event) {
  return { account: await routeQuery(event, new AccountStore()) };
}
