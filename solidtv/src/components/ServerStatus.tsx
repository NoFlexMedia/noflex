// SPDX-License-Identifier: AGPL-3.0-only
//
// The web's server status dot, for administrators: a dot for health, a ring per
// kind of work under way. The rings don't spin as they do on the web: anything
// moving keeps the renderer drawing every frame, which a TV pays for all evening.
// Polls only while the navigation is showing it, and never under the player.

import { createEffect, createSignal, For, onCleanup, Show, type Accessor } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { SERVER_ACTIVITY } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { player } from "$lib/playback.js";
import { app } from "$lib/session.js";
import { poll } from "$lib/ui.js";
import { color, motion, type } from "../theme.js";

const OPEN_MS = 5000;
const CLOSED_MS = 30_000;
const DOT = 16;
const RING_STEP = 12;

interface Status {
  systemStatus: { activeScans: number; plugins: { circuitOpen: number }; server: { database: { healthy: boolean } } };
  activeStreams: { id: string; transcode?: { status: string } | null }[];
  optimizedVersionStats: { running: number };
}

export default function ServerStatus(props: NodeProps & { open: Accessor<boolean> }) {
  const [status, setStatus] = createSignal<Status | null>(null);
  const [unreachable, setUnreachable] = createSignal(false);
  const admin = () => !!app.viewer?.isAdmin;

  const tick = () => {
    if (player.current) return;
    request<Status>(SERVER_ACTIVITY).then((data) => { setStatus(data); setUnreachable(false); }, () => setUnreachable(true));
  };
  createEffect(() => {
    if (!admin()) return;
    tick();
    onCleanup(poll(tick, props.open() ? OPEN_MS : CLOSED_MS));
  });

  const transcoding = () => status()?.activeStreams.filter((stream) => stream.transcode?.status === "running").length ?? 0;
  const playing = () => status()?.activeStreams.filter((stream) => stream.transcode?.status !== "running").length ?? 0;
  const scans = () => status()?.systemStatus.activeScans ?? 0;
  const optimizing = () => status()?.optimizedVersionStats.running ?? 0;
  const rings = () => {
    const active: { tone: number; label: string }[] = [];
    if (scans() > 0) active.push({ tone: color.focus, label: "Scanning" });
    if (transcoding() > 0) active.push({ tone: color.accent, label: `${transcoding()} transcoding` });
    if (playing() > 0) active.push({ tone: color.success, label: `${playing()} playing` });
    if (optimizing() > 0) active.push({ tone: color.text2, label: "Optimizing" });
    return active;
  };
  const problem = () => unreachable() ? "Can’t reach the server" : status()?.systemStatus.server.database.healthy === false ? "Database unhealthy" : (status()?.systemStatus.plugins.circuitOpen ?? 0) > 0 ? "Plugin trouble" : "";
  const tone = () => (unreachable() || status()?.systemStatus.server.database.healthy === false ? color.danger : problem() ? color.warning : color.success);
  const summary = () => problem() || rings().map((ring) => ring.label).join(" · ") || `v${app.server?.version ?? ""}`;
  const CENTER = 36;

  return (
    <view {...props} width={360} height={72} skipFocus>
      <Show when={admin()}>
        <view x={CENTER - DOT / 2} y={CENTER - DOT / 2} width={DOT} height={DOT} borderRadius={DOT / 2} color={tone()} transition={{ color: motion.out }} />
        <For each={rings()}>
          {(ring, index) => {
            const size = DOT + (index() + 1) * RING_STEP;
            return <view x={CENTER - size / 2} y={CENTER - size / 2} width={size} height={size} borderRadius={size / 2} color={color.clear} border={{ width: 3, color: ring.tone }} />;
          }}
        </For>
      </Show>
      <Show when={props.open()}>
        <text x={76} y={admin() ? 2 : 18} fontSize={type.caption} fontWeight={600} maxLines={1} contain="width" width={280}>{app.server?.name ?? "NoFlex"}</text>
        <Show when={admin()}>
          <text x={76} y={36} fontSize={22} color={color.text3} maxLines={1} contain="width" width={280}>{summary()}</text>
        </Show>
      </Show>
    </view>
  );
}

