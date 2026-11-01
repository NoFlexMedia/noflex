// SPDX-License-Identifier: AGPL-3.0-only
//
// Image URLs come back from GraphQL carrying a user-bound media token that
// is stable for an hour, so plain <img src> works with native lazy loading
// and the browser's HTTP cache. The server resizes on `w`/`h`/`fill`.

export type ArtKind = "poster" | "backdrop" | "thumb" | "logo";

const PREFERENCE: Record<ArtKind, string[]> = {
  poster: ["PRIMARY", "BOX", "THUMB", "ART"],
  backdrop: ["BACKDROP", "ART", "BANNER", "THUMB", "PRIMARY"],
  thumb: ["THUMB", "BACKDROP", "PRIMARY"],
  logo: ["LOGO"],
};

export function pickImage(images: { kind: string; url: string }[] | null | undefined, kind: ArtKind): string | null {
  if (!images?.length) return null;
  for (const name of PREFERENCE[kind]) {
    const hit = images.find((image) => image.kind === name);
    if (hit) return hit.url;
  }
  return kind === "logo" ? null : images[0]?.url ?? null;
}

export function sized(url: string, width: number, height?: number): string {
  const params = new URLSearchParams({ w: String(Math.round(width)) });
  if (height) { params.set("h", String(Math.round(height))); params.set("fill", "true"); }
  return `${url}${url.includes("?") ? "&" : "?"}${params}`;
}

/** `srcset` across density buckets for a box of `width` CSS px at `ratio` (h/w). */
export function srcset(url: string, width: number, ratio?: number): string {
  return [1, 1.5, 2]
    .map((density) => {
      const w = Math.round(width * density);
      return `${sized(url, w, ratio ? w * ratio : undefined)} ${w}w`;
    })
    .join(", ");
}

const ambientCache = new Map<string, string>();

/**
 * Sample a muted, saturated accent from an image as an "r g b" triplet, for
 * tinting page chrome. Same-origin, so the canvas is never tainted.
 */
export async function ambientColor(url: string): Promise<string | null> {
  const key = url.split("?")[0] ?? url;
  const cached = ambientCache.get(key);
  if (cached) return cached;
  const image = new Image();
  image.decoding = "async";
  image.src = sized(url, 48, 27);
  try { await image.decode(); } catch { return null; }
  const canvas = document.createElement("canvas");
  canvas.width = 24; canvas.height = 14;
  const context = canvas.getContext("2d", { willReadFrequently: true });
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
  const triplet = `${scale(r)} ${scale(g)} ${scale(b)}`;
  ambientCache.set(key, triplet);
  return triplet;
}
