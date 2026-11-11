// SPDX-License-Identifier: AGPL-3.0-only

import { SITE } from "$lib/site";

export const prerender = true;

const pages = ["/", "/download", "/demo", "/support", "/privacy", "/terms", "/delete-account", "/credits"];

export function GET() {
  const urls = pages.map((path) => `  <url><loc>${SITE}${path}</loc></url>`).join("\n");
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`;
  return new Response(body, { headers: { "content-type": "application/xml" } });
}
