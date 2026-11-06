// SPDX-License-Identifier: AGPL-3.0-only
//
// Image URLs come back from GraphQL carrying a media token that is stable for an
// hour, so the renderer's own loader needs no headers. The server resizes on
// `w`/`h`/`fill`, which is what keeps a wall of posters inside texture memory.

import { mediaUrl } from "$lib/tokens.js";
import type { Image } from "$lib/types.js";

export type ArtKind = "poster" | "backdrop" | "thumb" | "logo";

const PREFERENCE: Record<ArtKind, string[]> = {
  poster: ["PRIMARY", "BOX", "THUMB", "ART"],
  backdrop: ["BACKDROP", "ART", "BANNER", "THUMB", "PRIMARY"],
  thumb: ["THUMB", "BACKDROP", "PRIMARY"],
  logo: ["LOGO"],
};

export function pickImage(images: Image[] | null | undefined, kind: ArtKind): string | null {
  if (!images?.length) return null;
  for (const name of PREFERENCE[kind]) {
    const hit = images.find((image) => image.kind === name);
    if (hit) return mediaUrl(hit.url);
  }
  return kind === "logo" ? null : mediaUrl(images[0]?.url ?? null);
}

export function sized(url: string, width: number, height?: number): string {
  const size = `w=${Math.round(width)}${height ? `&h=${Math.round(height)}&fill=true` : ""}`;
  return `${url}${url.indexOf("?") >= 0 ? "&" : "?"}${size}`;
}

const ambientCache = new Map<string, number>();

/**
 * Sample a muted, saturated accent from an image, for tinting the page. Null
 * where there's no 2D canvas to read it with: the page keeps its default tint.
 */
export async function ambientColor(url: string): Promise<number | null> {
  const key = url.split("?")[0] ?? url;
  const cached = ambientCache.get(key);
  if (cached != null) return cached;
  if (__TARGET__ === "nativescript") return null;
  try {
    const image = new window.Image();
    image.crossOrigin = "anonymous";
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("no image"));
      image.src = sized(url, 48, 27);
    });
    const canvas = document.createElement("canvas");
    canvas.width = 24;
    canvas.height = 14;
    const context = canvas.getContext("2d");
    if (!context) return null;
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
    let r = 0, g = 0, b = 0, weight = 0;
    for (let index = 0; index < data.length; index += 4) {
      const pr = data[index] ?? 0, pg = data[index + 1] ?? 0, pb = data[index + 2] ?? 0;
      const max = Math.max(pr, pg, pb), min = Math.min(pr, pg, pb);
      // Favour saturated mid-tones; near-black and blown highlights say little.
      const w = ((max - min) / 255 + 0.08) * (max > 24 && min < 240 ? 1 : 0.05);
      r += pr * w; g += pg * w; b += pb * w; weight += w;
    }
    if (!weight) return null;
    const scale = (value: number) => Math.round(Math.min(200, Math.max(40, value / weight)));
    const tint = ((scale(r) << 24) | (scale(g) << 16) | (scale(b) << 8) | 0xff) >>> 0;
    ambientCache.set(key, tint);
    return tint;
  } catch {
    return null;
  }
}
