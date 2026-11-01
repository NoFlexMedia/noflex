// SPDX-License-Identifier: AGPL-3.0-only

import { prefersReducedMotion } from "$lib/ui.svelte.js";

const finePointer = () => typeof matchMedia !== "undefined" && matchMedia("(hover: hover) and (pointer: fine)").matches;

/**
 * Pointer-tracking 3D tilt with a specular highlight. Writes `--rx`, `--ry`,
 * `--gx`, `--gy` custom properties; CSS owns the actual transform so the
 * effect is free on touch devices and under reduced motion.
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

/** Invoke `callback` whenever the node scrolls within `margin` of the viewport. */
export function inview(node: HTMLElement, options: { callback: () => void; margin?: string }) {
  let current = options;
  const observer = new IntersectionObserver((entries) => {
    if (entries.some((entry) => entry.isIntersecting)) current.callback();
  }, { rootMargin: options.margin ?? "600px 0px" });
  observer.observe(node);
  return {
    update(next: typeof options) { current = next; },
    destroy() { observer.disconnect(); },
  };
}
