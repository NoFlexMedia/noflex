// SPDX-License-Identifier: AGPL-3.0-only
//
// The few patterns views share on top of Houdini's stores.

import type { LoadEvent } from "@sveltejs/kit";
import { cache, CachePolicy } from "$houdini";
import type { CachePolicies, DocumentArtifact, QueryResult } from "houdini/runtime";
import WorkUserData from "$houdini/artifacts/WorkUserData";
import client, { toError } from "../client.js";
import { hasSession } from "$lib/tokens.js";

type Errors = QueryResult["errors"];
interface Fetchable<Input> { fetch(params?: { event?: LoadEvent; variables?: Input; policy?: CachePolicies }): Promise<unknown> }

/** A result's errors as one message, or "" when it has none. */
export function failure(result: { errors?: Errors } | null | undefined): string {
  return result?.errors?.length ? toError(result.errors).message : "";
}

const ignoreAbort = (error: unknown) => {
  if (!(error instanceof Error && error.name === "AbortError")) throw error;
};

/**
 * Fetch a component's query under its document's cache policy. A request
 * cancelled by a newer one for the same data (Houdini's dedupe) is not an
 * error; failures land on the store.
 */
export function fetchQuery<Input>(store: Fetchable<Input>, variables?: Input): Promise<void> {
  return store.fetch({ variables }).then(() => {}, ignoreAbort);
}

/** Refetch past the cache — a retry button or a poll tick. */
export function refetch(store: Fetchable<never>): Promise<void> {
  return store.fetch({ policy: CachePolicy.NetworkOnly }).then(() => {}, ignoreAbort);
}

/**
 * Start a route's query from its `load`, so hover preloading warms it and
 * the page paints from the cache. Without a session the store idles; signing
 * in reruns every load.
 */
export async function routeQuery<Input, Store extends Fetchable<Input>>(event: LoadEvent, store: Store, variables?: Input): Promise<Store> {
  if (hasSession()) await store.fetch({ event, variables }).catch(ignoreAbort);
  return store;
}

/** Load a paginated store's next page; its errors throw. */
export async function nextPage(store: { loadNextPage(): Promise<{ errors?: Errors }> }): Promise<void> {
  const result = await store.loadNextPage().catch((error: unknown) => { ignoreAbort(error); return null; });
  if (result?.errors?.length) throw toError(result.errors);
}

export interface RequestOptions { policy?: CachePolicies; metadata?: App.Metadata }

/**
 * Send one operation outside a component's lifecycle and return its data.
 * Errors throw (see `toError`). The cache is read per `policy` and always
 * written, so whatever views show the result update too.
 */
export async function request<Data = any>(store: { artifact: DocumentArtifact }, variables: Record<string, unknown> = {}, { policy = CachePolicy.NetworkOnly, metadata }: RequestOptions = {}): Promise<Data> {
  const { artifact } = store;
  const observer = client.observe<any, any>({ artifact });
  try {
    const defaults = "input" in artifact ? artifact.input?.defaults : undefined;
    const result = await observer.send({ variables: { ...defaults, ...variables }, policy, metadata });
    if (result.errors?.length) throw toError(result.errors);
    return result.data as Data;
  } finally {
    void observer.cleanup();
  }
}

/** Show a playback-state mutation's result on its work wherever it's rendered. */
export function writeUserData(workId: string, userData: Record<string, unknown>): void {
  cache.get("Work", { id: workId }).write({ fragment: { artifact: WorkUserData }, data: { userData } as never });
}
