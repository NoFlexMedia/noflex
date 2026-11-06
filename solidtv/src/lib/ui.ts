// SPDX-License-Identifier: AGPL-3.0-only
//
// App-wide UI state: toasts, the page's backdrop and tint, and the helpers views share.

import { reactive } from "$lib/state.js";
import { AMBIENT } from "../theme.js";

export type ToastKind = "info" | "success" | "error";

export const ui = reactive({
  toast: null as { message: string; kind: ToastKind; key: number } | null,
  /** The art behind the page; a null keeps the page's plain dark. */
  backdrop: null as string | null,
  /** Quieten the art, for a page with a lot to read over it. */
  backdropDim: false,
  ambient: AMBIENT as number,
});

let toastTimer: ReturnType<typeof setTimeout> | undefined;
export function toast(message: string, kind: ToastKind = "info"): void {
  clearTimeout(toastTimer);
  ui.toast = { message, kind, key: Date.now() };
  toastTimer = setTimeout(() => { ui.toast = null; }, kind === "error" ? 6000 : 3500);
}

/** Run a user-triggered action; failures surface as a toast, and resolve false. */
export async function attempt(action: () => Promise<unknown>, success?: string): Promise<boolean> {
  try {
    await action();
    if (success) toast(success, "success");
    return true;
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
    return false;
  }
}

/** Call `tick` every `ms` while the app is on screen. Returns the stop function. */
export function poll(tick: () => void, ms: number): () => void {
  const timer = setInterval(() => {
    if (typeof document !== "undefined" && document.hidden) return;
    tick();
  }, ms);
  return () => clearInterval(timer);
}
