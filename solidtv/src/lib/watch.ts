// SPDX-License-Identifier: AGPL-3.0-only
//
// Progress and favorites as they change here, so cards update without refetching:
// the same job as the web's `writeUserData` into its normalized cache, or Flutter's
// WatchStateService. The page cache holds whole responses, so changes live beside it.

import { createSignal, type Signal } from "solid-js";
import type { UserData, Work } from "$lib/types.js";

const overrides = new Map<string, Signal<UserData | null>>();

function slot(id: string): Signal<UserData | null> {
  let entry = overrides.get(id);
  if (!entry) {
    entry = createSignal<UserData | null>(null);
    overrides.set(id, entry);
  }
  return entry;
}

export function setUserData(id: string, data: UserData): void { slot(id)[1](data); }

/** What's known of a work's play state; reactive. */
export function userDataOf(work: Pick<Work, "id" | "userData">): UserData | null {
  return slot(work.id)[0]() ?? work.userData ?? null;
}

export function resetWatch(): void { overrides.clear(); }
