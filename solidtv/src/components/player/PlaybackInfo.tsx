// SPDX-License-Identifier: AGPL-3.0-only
//
// How this is reaching the screen, as on the web: what the server decided and why, and
// what the decoder is doing. Polled only while it's open.

import { createSignal, For, onCleanup, onMount } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { codecName, levelName, profileName } from "$lib/format.js";
import type { Playback } from "$lib/playback.js";
import type { PlayerStats } from "$lib/player/types.js";
import type { StreamSummary } from "$lib/types.js";
import { color, radius, type, weight } from "../../theme.js";

const WIDTH = 860;
const LABEL = 170;
const PAD = 32;
const LINE = 36;
const VALUE_CHARS = 54;
const PLAN: Record<string, string> = { DIRECT_PLAY: "Direct play", REMUX: "Direct stream (remux)", TRANSCODE_AUDIO: "Transcoding audio", TRANSCODE_VIDEO: "Transcoding video", TRANSCODE: "Transcoding" };

const rate = (bps?: number | null) => (bps ? `${(bps / 1e6).toFixed(1)} Mb/s` : "");
const flow = (from: string, to: string) => (from === to ? from : `${from} → ${to}`);

function videoText(stream: StreamSummary): string {
  if (!stream.videoCodec) return "none";
  return [codecName(stream.videoCodec), profileName(stream.videoProfile), levelName(stream.videoCodec, stream.videoLevel), stream.width && `${stream.width}×${stream.height}`, stream.bitDepth && `${stream.bitDepth}-bit`, stream.videoRange && stream.videoRange !== "SDR" && stream.videoRange.replace(/_/g, " ")].filter(Boolean).join(" · ");
}
const audioText = (stream: StreamSummary) => (stream.audioCodec ? [stream.audioCodec.toUpperCase(), stream.audioChannels && `${stream.audioChannels} ch`].filter(Boolean).join(" · ") : "none");

export default function PlaybackInfo(props: NodeProps & { playback: Playback; stats: () => PlayerStats | null; buffered: () => number; live: boolean }) {
  const [stats, setStats] = createSignal<PlayerStats | null>(props.stats());
  const [ahead, setAhead] = createSignal(0);
  onMount(() => {
    const poll = setInterval(() => { setStats(props.stats()); setAhead(props.buffered()); }, 1000);
    onCleanup(() => clearInterval(poll));
  });

  const rows = () => {
    const decision = props.playback.decision;
    const live = stats();
    const list: [string, string][] = [["Mode", decision ? PLAN[decision.plan] ?? decision.plan : props.live ? "Live transcode" : "—"]];
    if (decision) {
      if (decision.reasons.length) list.push(["Why", decision.reasons.map((reason) => reason.description).join(" ")]);
      list.push(["Container", flow(decision.source.container ?? "?", decision.target.container ?? "?")]);
      list.push(["Video", flow(videoText(decision.source), videoText(decision.target))]);
      list.push(["Audio", flow(audioText(decision.source), audioText(decision.target))]);
      if (decision.encoder) list.push(["Encoder", [`${decision.encoder}${decision.hardware ? " (hardware)" : ""}`, decision.audioEncoder && `audio ${decision.audioEncoder}`, decision.tonemap && "HDR → SDR tonemap"].filter(Boolean).join(" · ")]);
      if (decision.target.bitrateBps) list.push(["Bitrate", rate(decision.target.bitrateBps)]);
    }
    if (live) {
      list.push(["Decoded", live.width ? `${live.width}×${live.height}` : "—"]);
      if (live.droppedFrames != null) list.push(["Dropped", `${live.droppedFrames}${live.totalFrames != null ? ` / ${live.totalFrames}` : ""}`]);
      list.push(["Buffer", `${ahead().toFixed(1)} s`]);
      if (live.bandwidthBps) list.push(["Bandwidth", `${rate(live.bandwidthBps)}${live.streamBitrateBps ? ` (stream ${rate(live.streamBitrateBps)})` : ""}`]);
    }
    // No text is measured ahead of drawing, so rows reserve lines by length.
    let y = PAD + 58;
    return list.map(([label, value]) => {
      const lines = Math.min(3, Math.ceil(value.length / VALUE_CHARS));
      const row = { label, value, lines, y };
      y += lines * LINE + 6;
      return row;
    });
  };
  const height = () => {
    const last = rows()[rows().length - 1];
    return last ? last.y + last.lines * LINE + PAD - 6 : PAD * 2;
  };

  return (
    <view {...props} width={WIDTH} height={height()} borderRadius={radius.l} color={0x0c0e12e6} border={{ width: 2, color: color.lineStrong }} skipFocus>
      <text x={PAD} y={PAD} fontSize={type.body} fontWeight={weight.semibold}>Playback info</text>
      <For each={rows()}>
        {(row) => (
          <>
            <text x={PAD} y={row.y} lineHeight={LINE} fontSize={22} color={color.text3}>{row.label}</text>
            <text x={PAD + LABEL} y={row.y} width={WIDTH - PAD * 2 - LABEL} contain="width" maxLines={row.lines} lineHeight={LINE} fontSize={22} color={color.text}>{row.value}</text>
          </>
        )}
      </For>
    </view>
  );
}
