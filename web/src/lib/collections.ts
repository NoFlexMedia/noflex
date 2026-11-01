// SPDX-License-Identifier: AGPL-3.0-only
//
// `setCollectionItems`/`setPlaylistItems` replace the whole list, so edits
// need every current entry; shelves only fetch a preview page.

import { CollectionItemIdsStore, PlaylistTargetsStore, SetCollectionItemsStore, SetPlaylistItemsStore } from "$houdini";
import type { DocumentArtifact } from "houdini/runtime";
import { request } from "$lib/graphql.js";

async function pages<T>(store: { artifact: DocumentArtifact }, id: string, pick: (data: any) => any): Promise<T[]> {
  const items: T[] = [];
  let after: string | null = null;
  do {
    const page: any = pick(await request(store, { id, after }));
    if (!page) break;
    items.push(...page.nodes);
    after = page.pageInfo.hasNextPage ? page.pageInfo.endCursor : null;
  } while (after);
  return items;
}

async function collectionWorkIds(id: string): Promise<string[]> {
  const items = await pages<{ id: string }>(new CollectionItemIdsStore(), id, (data) => data.collection?.items);
  return items.map((item) => item.id);
}

async function playlistTargets(id: string): Promise<{ kind: string; id: string }[]> {
  const items = await pages<any>(new PlaylistTargetsStore(), id, (data) => data.playlist?.items);
  return items.flatMap((item) => {
    const target = item.work?.id ?? item.edition?.id;
    return target ? [{ kind: item.kind, id: target }] : [];
  });
}

/** Append `workId` to a collection unless it's already a member. */
export async function addToCollection(collectionId: string, workId: string): Promise<void> {
  const ids = await collectionWorkIds(collectionId);
  if (!ids.includes(workId)) await request(new SetCollectionItemsStore(), { id: collectionId, items: [...ids, workId] });
}

export async function removeFromCollection(collectionId: string, workId: string): Promise<void> {
  const ids = await collectionWorkIds(collectionId);
  await request(new SetCollectionItemsStore(), { id: collectionId, items: ids.filter((id) => id !== workId) });
}

/** Drop the first playlist entry pointing at `targetId`. */
export async function removeFromPlaylist(playlistId: string, targetId: string): Promise<void> {
  const targets = await playlistTargets(playlistId);
  const drop = targets.findIndex((target) => target.id === targetId);
  await request(new SetPlaylistItemsStore(), { id: playlistId, items: targets.filter((_, position) => position !== drop) });
}
