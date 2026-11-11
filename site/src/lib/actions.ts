// SPDX-License-Identifier: AGPL-3.0-only

export const prefersReducedMotion = () => typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
const finePointer = () => typeof matchMedia !== "undefined" && matchMedia("(hover: hover) and (pointer: fine)").matches;

/**
 * Pointer-tracking 3D tilt with a specular highlight, as `web/`'s `tilt`:
 * writes `--rx`, `--ry`, `--gx`, `--gy` and leaves the transform to CSS.
 */
export function tilt(node: HTMLElement, max = 7) {
  if (!finePointer() || prefersReducedMotion()) return {};
  let frame = 0;
  let rect: DOMRect | null = null;
  const enter = () => { rect = node.getBoundingClientRect(); node.dataset.tilting = ""; };
  const move = (event: PointerEvent) => {
    if (!rect) rect = node.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width;
    const y = (event.clientY - rect.top) / rect.height;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      node.style.setProperty("--ry", `${(x - 0.5) * max * 2}deg`);
      node.style.setProperty("--rx", `${(0.5 - y) * max * 2}deg`);
      node.style.setProperty("--gx", `${x * 100}%`);
      node.style.setProperty("--gy", `${y * 100}%`);
    });
  };
  const leave = () => {
    cancelAnimationFrame(frame);
    rect = null;
    delete node.dataset.tilting;
    for (const name of ["--rx", "--ry", "--gx", "--gy"]) node.style.removeProperty(name);
  };
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointermove", move);
  node.addEventListener("pointerleave", leave);
  return {
    destroy() {
      leave();
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerleave", leave);
    },
  };
}

const revealed = new WeakMap<Element, () => void>();
let revealer: IntersectionObserver | null = null;

/**
 * Fades the node up once it scrolls into view, then calls `onreveal`. Prerendered
 * HTML carries no `data-reveal`, so the page reads fine before (or without) JS.
 */
export function reveal(node: HTMLElement, onreveal?: () => void) {
  if (prefersReducedMotion()) { onreveal?.(); return {}; }
  revealer ??= new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const target = entry.target as HTMLElement;
      target.dataset.reveal = "in";
      revealed.get(target)?.();
      revealer?.unobserve(target);
    }
  }, { rootMargin: "0px 0px -12% 0px" });
  // Already on screen at load, as the hero is: no need to hide it first.
  const rect = node.getBoundingClientRect();
  if (rect.top < innerHeight * 0.88 && rect.bottom > 0) { onreveal?.(); return {}; }
  node.dataset.reveal = "";
  if (onreveal) revealed.set(node, onreveal);
  revealer.observe(node);
  return { destroy() { revealer?.unobserve(node); revealed.delete(node); } };
}

/** 0 as the node's top enters the bottom of the viewport, 1 as its bottom leaves the top. */
export function scrollProgress(node: HTMLElement, callback: (progress: number) => void) {
  let frame = 0;
  const update = () => {
    frame = 0;
    const rect = node.getBoundingClientRect();
    const span = rect.height + innerHeight;
    callback(Math.min(1, Math.max(0, (innerHeight - rect.top) / span)));
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  let visible = false;
  const observer = new IntersectionObserver(([entry]) => {
    visible = !!entry?.isIntersecting;
    if (visible) { addEventListener("scroll", schedule, { passive: true }); schedule(); }
    else removeEventListener("scroll", schedule);
  });
  observer.observe(node);
  addEventListener("resize", schedule);
  update();
  return {
    destroy() {
      cancelAnimationFrame(frame);
      observer.disconnect();
      removeEventListener("scroll", schedule);
      removeEventListener("resize", schedule);
    },
  };
}

/** Starts a button's press highlight (`.btn::after`) where the pointer went down. */
export function trackPress(event: PointerEvent) {
  const button = (event.target as Element | null)?.closest<HTMLElement>(".btn");
  if (!button) return;
  const rect = button.getBoundingClientRect();
  button.style.setProperty("--px", `${event.clientX - rect.left}px`);
  button.style.setProperty("--py", `${event.clientY - rect.top}px`);
}
