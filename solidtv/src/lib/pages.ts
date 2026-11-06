// SPDX-License-Identifier: AGPL-3.0-only

import { createEffect, createSignal, on } from "solid-js";
import { request, type Document, type Vars } from "$lib/graphql.js";
import type { Connection, Work } from "$lib/types.js";

/** A grid's works, a page at a time as focus nears the end. Reset when `variables` change. */
export function createPagedWorks(document: Document, variables: () => Vars | null) {
  const [items, setItems] = createSignal<Work[]>([]);
  const [error, setError] = createSignal("");
  const [fetching, setFetching] = createSignal(true);
  let cursor: string | null = null;
  let more = true;
  let ticket = 0;

  const load = async () => {
    const input = variables();
    if (!input || !more) return;
    const run = ++ticket;
    setFetching(true);
    try {
      const data: { works: Connection<Work> } = await request(document, { ...input, after: cursor });
      if (run !== ticket) return;
      setItems((known) => known.concat(data.works.nodes));
      cursor = data.works.pageInfo?.endCursor ?? null;
      more = !!data.works.pageInfo?.hasNextPage;
      setError("");
    } catch (cause) {
      if (run === ticket) setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (run === ticket) setFetching(false);
    }
  };

  const reset = () => { cursor = null; more = true; setItems([]); return load(); };
  createEffect(on(() => JSON.stringify(variables()), () => { void reset(); }));
  return { items, error, fetching, loadMore: () => { if (!fetching()) void load(); }, retry: reset };
}
