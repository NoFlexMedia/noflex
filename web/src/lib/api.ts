// SPDX-License-Identifier: AGPL-3.0-only

import { browser } from "$app/environment";
import { activeGraphqlUrl, activeHeaders, authHeaders, currentAccessToken, hasSession, refreshAccessToken } from "$lib/tokens.js";

/** A human-readable device label for the sessions list, e.g. "Firefox on Linux". */
export function deviceName(): string {
  if (!browser) return "Browser";
  const ua = navigator.userAgent;
  const browserName = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Browser";
  const os = /Windows/.test(ua) ? "Windows" : /Android/.test(ua) ? "Android" : /iPhone|iPad/.test(ua) ? "iOS" : /Mac OS X/.test(ua) ? "macOS" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${browserName} on ${os}` : browserName;
}

/** Authenticated GET for byte routes that don't take a media token. */
export async function authFetch(url: string, signal?: AbortSignal): Promise<Response> {
  const sentAccess = currentAccessToken();
  const response = await fetch(url, { headers: authHeaders(), signal });
  if (response.status === 401 && hasSession()) {
    await refreshAccessToken(sentAccess);
    return fetch(url, { headers: authHeaders(), signal });
  }
  return response;
}

const NONCE_KEY = "noflex.externalNonce";

/**
 * Mint the per-flow nonce that binds an external login/link to this tab;
 * the server refuses to redeem a callback code without it.
 */
export function newExternalNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  const nonce = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  sessionStorage.setItem(NONCE_KEY, nonce);
  return nonce;
}

/** Consume a callback code from the URL fragment, with the nonce that started its flow. */
export function takeExternalCallback(): { kind: "login" | "link"; code: string; nonce: string } | null {
  const match = location.hash.match(/^#external-(code|link-code)=(.+)$/);
  if (!match?.[2]) return null;
  history.replaceState(null, "", `${location.pathname}${location.search}`);
  const nonce = sessionStorage.getItem(NONCE_KEY) ?? "";
  sessionStorage.removeItem(NONCE_KEY);
  return { kind: match[1] === "code" ? "login" : "link", code: decodeURIComponent(match[2]), nonce };
}

/** Send `store`'s operation in a request that survives page unload; fire and forget. */
export function reportOnExit(store: { artifact: { name: string; raw: string } }, variables: Record<string, unknown>): void {
  if (!currentAccessToken()) return;
  const body = JSON.stringify({ operationName: store.artifact.name, query: store.artifact.raw, variables });
  fetch(activeGraphqlUrl(), { method: "POST", headers: { "Content-Type": "application/json", ...activeHeaders() }, body, keepalive: true }).catch(() => {});
}

/** `url` if it is an absolute http(s) URL, else null — for links that plugins supply. */
export function webUrl(url: string | null | undefined): string | null {
  if (!url) return null;
  try { return /^https?:$/.test(new URL(url).protocol) ? url : null; } catch { return null; }
}

/** The current access token's claims: `sub` is the user's id, `sid` the session's. Empty when signed out. */
export function accessClaims(): { sub?: string; sid?: string } {
  const payload = currentAccessToken()?.split(".")[1];
  if (!payload) return {};
  try {
    const json = JSON.parse(atob(payload.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(payload.length / 4) * 4, "=")));
    return { sub: typeof json.sub === "string" ? json.sub : undefined, sid: typeof json.sid === "string" ? json.sid : undefined };
  } catch {
    return {};
  }
}
