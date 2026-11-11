// SPDX-License-Identifier: AGPL-3.0-only
//
// The Cast receiver must be served over HTTPS from the NoFlex domain
// (cast/AGENTS.md), so the site carries it at /cast/receiver/, copied as is.

import { cpSync } from "node:fs";

cpSync(new URL("../../cast/receiver", import.meta.url), new URL("../build/cast/receiver", import.meta.url), { recursive: true });
