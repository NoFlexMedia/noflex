// SPDX-License-Identifier: AGPL-3.0-only

import { setClientStarted } from "$houdini";

// A client-only SPA: even the first navigation's loads should paint from
// the cache and fetch in the background rather than hold the boot screen.
setClientStarted();
