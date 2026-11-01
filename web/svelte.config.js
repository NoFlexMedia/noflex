// SPDX-License-Identifier: AGPL-3.0-only

import adapter from "@sveltejs/adapter-static";

export default {
  kit: {
    adapter: adapter({ fallback: "index.html", strict: true }),
    alias: { $components: "src/components", $houdini: ".houdini", "$houdini/*": ".houdini/*" },
    csp: {
      mode: "hash",
      directives: {
        "default-src": ["self"],
        "base-uri": ["none"],
        "form-action": ["self"],
        "frame-ancestors": ["none"],
        "object-src": ["none"],
        "script-src": ["self"],
        "style-src": ["self"],
        "img-src": ["self", "blob:"],
        "media-src": ["self", "blob:"],
        "connect-src": ["self"],
      },
    },
  },
};
