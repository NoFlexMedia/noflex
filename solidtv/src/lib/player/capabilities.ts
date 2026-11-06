// SPDX-License-Identifier: AGPL-3.0-only
//
// What this engine can decode, from the web client's probe: the web hosts are
// browsers, so measuring them is the same job. One difference: the probe's
// `subtitleCodecs` asks for a <track> element, and we draw cues ourselves.

import { clientCapabilities as probe, type ClientCapabilities } from "$codecs";

export function clientCapabilities(): Promise<ClientCapabilities> {
  return probe().then((capabilities) => ({ ...capabilities, subtitleCodecs: ["WEB_VTT"] as ClientCapabilities["subtitleCodecs"] }));
}
