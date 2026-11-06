// SPDX-License-Identifier: AGPL-3.0-only
//
// An object whose fields are signals, read and written as plain properties:
// the web's `$state({...})`. No store, because Solid's stores are Proxies and
// webOS 3's Chrome 38 has none.

import { createSignal } from "solid-js";

export function reactive<T extends object>(initial: T): T {
  const target = {} as T;
  for (const key of Object.keys(initial) as (keyof T & string)[]) {
    const [read, write] = createSignal(initial[key]);
    Object.defineProperty(target, key, { enumerable: true, get: read, set: (value: T[typeof key]) => void write(() => value) });
  }
  return target;
}
