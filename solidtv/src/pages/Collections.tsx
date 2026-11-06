// SPDX-License-Identifier: AGPL-3.0-only

import { Column } from "@solidtv/solid/primitives";
import { createEffect, createSignal, For, Show } from "solid-js";
import { COLLECTIONS } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { app } from "$lib/session.js";
import type { Connection, Work } from "$lib/types.js";
import { PAGE_WIDTH, SCREEN } from "../theme.js";
import Page from "$components/Page.js";
import Rail, { railHeight } from "$components/Rail.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";

interface Shelf { id: string; title: string; items: Connection<Work> }

export default function Collections() {
  const [shelves, setShelves] = createSignal<Shelf[] | null>(null);
  const [error, setError] = createSignal("");

  // Collections live per library, and a request's complexity budget is about one page of shelves.
  const shelvesOf = async (library: string): Promise<Shelf[]> => {
    const found: Shelf[] = [];
    let after: string | null = null;
    do {
      const data: { collections: Connection<Shelf> } = await request(COLLECTIONS, { library, after });
      found.push(...data.collections.nodes);
      after = data.collections.pageInfo?.hasNextPage ? data.collections.pageInfo.endCursor ?? null : null;
    } while (after);
    return found;
  };
  const load = async () => {
    setError("");
    try {
      const answers = await Promise.all(app.libraries.map((library) => shelvesOf(library.id)));
      setShelves(answers.flat().filter((shelf) => shelf.items.nodes.length));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };
  createEffect(() => { void load(); });

  return (
    <Page title="Collections">
      <Show when={shelves()} fallback={
        <Show when={error()} fallback={<Skeleton y={0} />}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="Collections are out of reach" message={error()} action={() => void load()} />
        </Show>
      }>
        <Show when={shelves()!.length} fallback={<State areaWidth={PAGE_WIDTH} autofocus icon="cards-three" title="No collections yet" message="Make one from a browser, and it shows up here." />}>
          <Column autofocus width={PAGE_WIDTH} height={SCREEN.height * 2} scroll="always" gap={8}>
            <For each={shelves()!}>{(shelf) => <Rail title={shelf.title} items={shelf.items.nodes} height={railHeight()} />}</For>
          </Column>
        </Show>
      </Show>
    </Page>
  );
}
