// SPDX-License-Identifier: AGPL-3.0-only
//
// WebVTT, parsed here so cues draw on the canvas like everything else. <track>
// only exists on the web hosts, and AVPlayer takes no sidecar without a manifest
// rewrite, so one path covers every host and the styling can't differ.

export interface Cue { start: number; end: number; text: string }

const STAMP = /(?:(\d+):)?(\d{2}):(\d{2})[.,](\d{3})/;

function seconds(stamp: string): number {
  const match = STAMP.exec(stamp);
  if (!match) return Number.NaN;
  return Number(match[1] ?? 0) * 3600 + Number(match[2]) * 60 + Number(match[3]) + Number(match[4]) / 1000;
}

/** Cues in start order. Markup goes, as the renderer draws one style per text node. */
export function parseVtt(source: string): Cue[] {
  const cues: Cue[] = [];
  for (const block of source.replace(/\r\n?/g, "\n").split(/\n{2,}/)) {
    const lines = block.split("\n");
    const at = lines.findIndex((line) => line.indexOf("-->") >= 0);
    if (at < 0) continue;
    const [from = "", to = ""] = lines[at]!.split("-->");
    const start = seconds(from);
    const end = seconds(to);
    if (Number.isNaN(start) || Number.isNaN(end)) continue;
    const text = lines.slice(at + 1).join("\n").replace(/<[^>]+>/g, "").replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&nbsp;/g, " ").trim();
    if (text) cues.push({ start, end, text });
  }
  return cues.sort((a, b) => a.start - b.start);
}

/**
 * The cue text at `time`, overlapping cues stacked. Playback asks several times a
 * second over a film's worth of cues, so `from` carries the search on from the
 * last answer, and only a seek backwards walks far.
 */
export function cueAt(cues: readonly Cue[], time: number, from = 0): { text: string; index: number } {
  let index = Math.min(from, Math.max(0, cues.length - 1));
  while (index > 0 && (cues[index]?.start ?? 0) > time) index -= 1;
  while (index < cues.length - 1 && (cues[index + 1]?.start ?? Infinity) <= time) index += 1;
  const showing: string[] = [];
  for (let at = index; at >= 0 && at >= index - 3; at -= 1) {
    const cue = cues[at]!;
    if (cue.start <= time && time < cue.end) showing.unshift(cue.text);
  }
  return { text: showing.join("\n"), index };
}
