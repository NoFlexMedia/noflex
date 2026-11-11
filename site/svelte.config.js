// SPDX-License-Identifier: AGPL-3.0-only

import adapter from "@sveltejs/adapter-static";

export default {
  kit: {
    // Every page is prerendered: the site is plain files on any static host.
    // The fallback is what a host serves for a path that doesn't exist.
    adapter: adapter({ strict: true, fallback: "404.html" }),
    alias: { $components: "src/lib/components" },
    csp: {
      mode: "hash",
      directives: {
        "default-src": ["self"],
        "base-uri": ["none"],
        "form-action": ["none"],
        "object-src": ["none"],
        "script-src": ["self"],
        "style-src": ["self"],
        // Prerendering writes `style:` directives as attributes. No page carries user content.
        "style-src-attr": ["unsafe-inline"],
        "img-src": ["self"],
        "font-src": ["self"],
        "connect-src": ["self"],
      },
    },
  },
};
