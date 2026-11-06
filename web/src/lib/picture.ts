// SPDX-License-Identifier: AGPL-3.0-only
//
// Profile pictures. The browser sends a square small enough not to matter on a phone
// connection, or the file as it is when it can't read it (HEIC outside Safari, AVIF on
// an old one), which the server can. The server re-encodes it either way.

const SIDE = 1024;

/** A centred square of `file`, as the base64 JPEG `setProfilePicture` takes. */
export async function pictureForUpload(file: File): Promise<string> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return fileAsBase64(file);
  }
  const crop = Math.min(bitmap.width, bitmap.height);
  const side = Math.min(crop, SIDE);
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = side;
  canvas.getContext("2d")!.drawImage(bitmap, (bitmap.width - crop) / 2, (bitmap.height - crop) / 2, crop, crop, 0, 0, side, side);
  bitmap.close();
  const url = canvas.toDataURL("image/jpeg", 0.9);
  return url.slice(url.indexOf(",") + 1);
}

function fileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** "Vera Viewer" is VV, "vera" is VE. */
export function initials(name?: string | null): string {
  const words = (name ?? "").trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? [words[0]!, words[words.length - 1]!].map((word) => Array.from(word)[0]) : Array.from(words[0] ?? "").slice(0, 2);
  return letters.join("").toUpperCase();
}
