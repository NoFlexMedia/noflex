// SPDX-License-Identifier: AGPL-3.0-only

import { authFetch } from "$lib/api.js";

function filenameFrom(response: Response, fallback: string): string {
  const header = response.headers.get("content-disposition") ?? "";
  const match = header.match(/filename\*=UTF-8''([^;]+)|filename="?([^";]+)"?/i);
  const raw = match?.[1] ?? match?.[2];
  if (!raw) return fallback;
  try { return decodeURIComponent(raw); } catch { return raw; }
}

/**
 * Download an authenticated byte route to disk. Streams straight to a file
 * where the File System Access API exists, so multi-GB exports never sit in
 * memory; elsewhere falls back to a blob link. Resolves false if the user
 * dismissed the save picker.
 */
export async function saveFile(url: string, fallbackName: string): Promise<boolean> {
  const response = await authFetch(url);
  if (!response.ok) throw new Error(`Download failed (${response.status}).`);
  const name = filenameFrom(response, fallbackName);
  if (window.showSaveFilePicker && response.body) {
    let handle;
    try { handle = await window.showSaveFilePicker({ suggestedName: name }); }
    catch (cause) {
      await response.body.cancel().catch(() => {});
      if (cause instanceof DOMException && cause.name === "AbortError") return false;
      throw cause;
    }
    await response.body.pipeTo(await handle.createWritable());
    return true;
  }
  const href = URL.createObjectURL(await response.blob());
  const link = document.createElement("a");
  link.href = href;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(href), 60_000);
  return true;
}
