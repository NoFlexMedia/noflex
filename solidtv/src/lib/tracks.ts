// SPDX-License-Identifier: AGPL-3.0-only
//
// Which audio and subtitle tracks to start a title on, ported from
// web/src/lib/tracks.ts: the viewer's preferred languages, else the file's default.

import { audioFormat, languageCode, languageName } from "$lib/format.js";
import type { SubtitleTrack } from "$lib/types.js";

export interface PlaybackPrefs { audioLanguages: string[]; subtitlesOn: boolean; subtitleLanguages: string[]; quality: string; autoplayNext: boolean }

export const noPrefs: PlaybackPrefs = { audioLanguages: [], subtitlesOn: false, subtitleLanguages: [], quality: "auto", autoplayNext: true };

export function readPlaybackPrefs(value: any): PlaybackPrefs {
  return {
    audioLanguages: value?.audio?.languages ?? [],
    subtitlesOn: Boolean(value?.subtitles?.enabled),
    subtitleLanguages: value?.subtitles?.languages ?? [],
    quality: String(value?.quality ?? "auto"),
    autoplayNext: value?.autoplayNext ?? true,
  };
}

const speaks = (track: { language?: string | null }, language: string) => !!track.language && languageCode(track.language) === languageCode(language);

export function preferredAudio(streams: any[] | null | undefined, prefs: PlaybackPrefs): number | null {
  if (!streams?.length) return null;
  const indexed = streams.map((stream, index) => ({ ...stream, index }));
  for (const language of prefs.audioLanguages) {
    const matching = indexed.filter((stream) => speaks(stream, language));
    if (matching.length) return (matching.find((stream) => stream.default) ?? matching[0]).index;
  }
  return (indexed.find((stream) => stream.default) ?? indexed[0]).index;
}

export function preferredSubtitle(tracks: SubtitleTrack[] | null | undefined, prefs: PlaybackPrefs): number | null {
  if (!tracks?.length || !prefs.subtitlesOn) return null;
  const indexed = tracks.map((track, index) => ({ ...track, index }));
  for (const language of prefs.subtitleLanguages) {
    const matching = indexed.filter((track) => speaks(track, language) && !track.forced);
    if (matching.length) return matching[0].index;
  }
  return prefs.subtitleLanguages.length ? null : (indexed.find((track) => track.default && !track.forced) ?? null)?.index ?? null;
}

/** Image subtitles have no text to send, so they can only be burned in. */
export const isImageSubtitle = (track: SubtitleTrack | null | undefined) => /pgs|dvd|dvb|vobsub|xsub/i.test(track?.codec ?? "");

export interface Choice { value: string; label: string; hint?: string }

export function audioChoices(streams: any[] | null | undefined): Choice[] {
  return (streams ?? []).map((stream, index) => ({
    value: String(index),
    label: languageName(stream.language) || stream.title || `Track ${index + 1}`,
    hint: [languageName(stream.language) && stream.title, audioFormat(stream)].filter(Boolean).join(" · "),
  }));
}

export function subtitleChoices(tracks: SubtitleTrack[] | null | undefined): Choice[] {
  return [
    { value: "off", label: "Off" },
    ...(tracks ?? []).map((track, index) => ({
      value: String(index),
      label: languageName(track.language) || track.title || `Track ${index + 1}`,
      hint: [languageName(track.language) && track.title, track.forced && "Forced", isImageSubtitle(track) && "Burned in"].filter(Boolean).join(" · "),
    })),
  ];
}
