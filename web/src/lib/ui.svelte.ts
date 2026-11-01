// SPDX-License-Identifier: AGPL-3.0-only
//
// App-wide UI primitives that any view can reach without prop drilling:
// toasts, the confirm dialog, and visibility-aware polling.

export type ToastKind = "info" | "success" | "error";
export interface Toast { id: number; kind: ToastKind; message: string; action?: { label: string; run: () => void } }

let nextToast = 1;
export const toasts = $state<Toast[]>([]);

export function toast(message: string, kind: ToastKind = "info", action?: Toast["action"]): void {
  const id = nextToast++;
  toasts.push({ id, kind, message, action });
  if (toasts.length > 4) toasts.shift();
  setTimeout(() => dismissToast(id), kind === "error" ? 7000 : 4200);
}

export function dismissToast(id: number): void {
  const index = toasts.findIndex((item) => item.id === id);
  if (index !== -1) toasts.splice(index, 1);
}

/** Run a mutation-like task and surface the outcome as a toast. */
export async function attempt<T>(task: () => Promise<T>, success?: string): Promise<T | null> {
  try {
    const result = await task();
    if (success) toast(success, "success");
    return result;
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
    return null;
  }
}

export interface ConfirmRequest {
  title: string;
  message?: string;
  confirmLabel?: string;
  danger?: boolean;
  /** Optional checkbox, e.g. "Also delete files from disk". */
  option?: string;
  resolve: (result: { confirmed: boolean; option: boolean }) => void;
}

export const confirmState = $state<{ current: ConfirmRequest | null }>({ current: null });

export function ask(request: Omit<ConfirmRequest, "resolve">): Promise<{ confirmed: boolean; option: boolean }> {
  confirmState.current?.resolve({ confirmed: false, option: false });
  return new Promise((resolve) => { confirmState.current = { ...request, resolve }; });
}

export async function confirmAction(request: Omit<ConfirmRequest, "resolve">): Promise<boolean> {
  return (await ask(request)).confirmed;
}

/**
 * Call `task` every `ms` while the document is visible, and once
 * immediately when it becomes visible again. Returns a disposer.
 */
export function poll(task: () => unknown, ms: number): () => void {
  let timer: ReturnType<typeof setInterval> | undefined;
  const start = () => { stop(); timer = setInterval(task, ms); };
  const stop = () => { if (timer) clearInterval(timer); timer = undefined; };
  const onVisibility = () => { if (document.hidden) stop(); else { task(); start(); } };
  document.addEventListener("visibilitychange", onVisibility);
  if (!document.hidden) start();
  return () => { stop(); document.removeEventListener("visibilitychange", onVisibility); };
}

export const prefersReducedMotion = () => typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Artwork-derived tint for the page glow, as "r g b"; null restores the default. */
export const ambient = $state<{ color: string | null }>({ color: null });

/** The command palette's open state, so any view can summon it. */
export const palette = $state({ open: false });

const SIDEBAR_KEY = "noflex.sidebar";

/**
 * The desktop sidebar's collapsed state, remembered per browser and
 * mirrored onto `<html data-sidebar>` so layout CSS can follow it.
 * Below the desktop breakpoint the sidebar is always a rail.
 */
export const sidebar = $state({
  collapsed: (() => { try { return localStorage.getItem(SIDEBAR_KEY) === "collapsed"; } catch { return false; } })(),
});
if (typeof document !== "undefined") document.documentElement.dataset.sidebar = sidebar.collapsed ? "collapsed" : "expanded";

let sidebarSettle: ReturnType<typeof setTimeout> | undefined;

/**
 * Toggle the sidebar. Content beside it (`.nav-follow`) takes its new
 * layout once and slides there by transform, measured from where it is
 * on screen, so a toggle mid-slide reverses smoothly instead of
 * reflowing the page every frame.
 */
export function toggleSidebar(): void {
  const root = document.documentElement;
  const followers = [...document.querySelectorAll<HTMLElement>(".nav-follow")];
  const before = followers.map((el) => el.getBoundingClientRect().left);
  for (const el of followers) for (const animation of el.getAnimations()) animation.cancel();
  const animate = !prefersReducedMotion() && matchMedia("(min-width: 1180px)").matches;
  const style = getComputedStyle(root);
  const duration = parseFloat(style.getPropertyValue("--nav-t")) || 280;

  // The sidebar's and topbar's own transitions are gated on this flag so
  // crossing a breakpoint by resizing doesn't animate. It must land in the
  // same style change as the new state, or they snap.
  if (animate) {
    root.dataset.sidebarMotion = "";
    clearTimeout(sidebarSettle);
    sidebarSettle = setTimeout(() => delete root.dataset.sidebarMotion, duration + 50);
  }
  sidebar.collapsed = !sidebar.collapsed;
  root.dataset.sidebar = sidebar.collapsed ? "collapsed" : "expanded";
  try { localStorage.setItem(SIDEBAR_KEY, sidebar.collapsed ? "collapsed" : "expanded"); } catch { /* storage unavailable */ }
  if (!animate) return;

  const easing = style.getPropertyValue("--ease-out").trim() || "ease-out";
  followers.forEach((el, index) => {
    const dx = (before[index] ?? 0) - el.getBoundingClientRect().left;
    if (dx) el.animate([{ transform: `translateX(${dx}px)` }, { transform: "none" }], { duration, easing });
  });
}
