// SPDX-License-Identifier: AGPL-3.0-only
//
// What people can filter by, as the filters present it: the server's facets in
// words, the operators each one makes sense with, and how their inputs become a
// rule's values.

import { codecName, languageName } from "$lib/format.js";

export type MatchMode = "ALL" | "ANY" | "NONE";
/** What a condition's value input looks like. */
export type InputKind = "pick" | "text" | "number" | "days" | "year" | "years" | "none";

export interface Operator {
  id: string;
  label: string;
  match: MatchMode;
  input: InputKind;
  /** One line on what it does, for people meeting it for the first time. */
  hint?: string;
  /** A rule's inputs as the server's values; empty until they're complete. */
  encode?: (inputs: string[]) => string[];
}

export interface Field {
  facet: string;
  label: string;
  section: string;
  operators: Operator[];
  /** One line on what this looks at, where the name doesn't say. */
  hint?: string;
  /** Values the server doesn't list, because they're the same everywhere. */
  fixed?: { value: string; label: string }[];
  /** How to show a value the server names only by code. */
  label_of?: (value: string) => string;
  /** Always offered in the simple filters, rather than under "More". */
  quick?: boolean;
  /** Short name for its pill, when the field's reads long there. */
  pill?: string;
}

const ANY: Operator = { id: "any", label: "is any of", match: "ANY", input: "pick", hint: "Titles with at least one of these." };
const EVERY: Operator = { id: "all", label: "has all of", match: "ALL", input: "pick", hint: "Only titles with every one of these." };
const NONE: Operator = { id: "none", label: "is none of", match: "NONE", input: "pick", hint: "Leaves out titles with any of these." };
const MANY = [ANY, EVERY, NONE];
const ONE = [ANY, NONE];
const FILES = "Counts if any version of a title has it, or any episode of a show.";

const titleCaseWord = (text: string) => text.replace(/^\w/, (letter) => letter.toUpperCase());

const AUDIO_CODECS: Record<string, string> = { aac: "AAC", ac3: "Dolby Digital", eac3: "Dolby Digital Plus", truehd: "Dolby TrueHD", dts: "DTS", flac: "FLAC", opus: "Opus", mp3: "MP3", vorbis: "Vorbis" };
const SUBTITLE_CODECS: Record<string, string> = { sub_rip: "SRT", subrip: "SRT", srt: "SRT", ass: "ASS", ssa: "SSA", web_vtt: "WebVTT", webvtt: "WebVTT", vtt: "WebVTT", ttml: "TTML", mov_text: "MP4 text", pgs_image: "PGS", dvb_sub: "DVB" };
const RESOLUTIONS: Record<string, string> = { "8k": "8K", "4k": "4K", "1440p": "1440p", "1080p": "1080p", "720p": "720p", sd: "SD" };
const RANGES: Record<string, string> = { dolby_vision: "Dolby Vision", hdr10: "HDR10", hlg: "HLG", sdr: "SDR" };
const LAYOUTS: Record<string, string> = { atmos: "Dolby Atmos", "7.1": "7.1 surround", "5.1": "5.1 surround", stereo: "Stereo", mono: "Mono" };

const trimmed = (inputs: string[]) => (inputs[0]?.trim() ? [inputs[0].trim()] : []);

export const FIELDS: Field[] = [
  {
    facet: "WATCH_STATE",
    label: "Watch state",
    pill: "Watched",
    section: "You",
    quick: true,
    operators: ONE,
    hint: "A show is watched once every episode is, and in progress from the first one you start.",
    fixed: [{ value: "unwatched", label: "Unwatched" }, { value: "in_progress", label: "In progress" }, { value: "watched", label: "Watched" }],
  },
  {
    facet: "FAVORITE",
    label: "Favorite",
    section: "You",
    operators: [
      { id: "yes", label: "is a favorite", match: "ANY", input: "none", encode: () => ["yes"] },
      { id: "no", label: "isn’t a favorite", match: "NONE", input: "none", encode: () => ["yes"] },
    ],
  },
  { facet: "GENRE", label: "Genre", section: "About the title", quick: true, operators: MANY },
  {
    facet: "TITLE",
    label: "Title",
    section: "About the title",
    operators: [
      { id: "contains", label: "contains", match: "ANY", input: "text", encode: trimmed },
      { id: "excludes", label: "doesn’t contain", match: "NONE", input: "text", encode: trimmed },
    ],
  },
  {
    facet: "YEARS",
    label: "Release year",
    pill: "Decade",
    section: "About the title",
    quick: true,
    operators: [
      { id: "any", label: "is in", match: "ANY", input: "pick" },
      { id: "none", label: "isn’t in", match: "NONE", input: "pick" },
      { id: "from", label: "is from", match: "ANY", input: "year", hint: "This year or later.", encode: (inputs) => (inputs[0] ? [`${inputs[0]}-`] : []) },
      { id: "until", label: "is until", match: "ANY", input: "year", hint: "This year or earlier.", encode: (inputs) => (inputs[0] ? [`-${inputs[0]}`] : []) },
      { id: "between", label: "is between", match: "ANY", input: "years", encode: (inputs) => (inputs[0] && inputs[1] ? [`${inputs[0]}-${inputs[1]}`] : []) },
    ],
  },
  {
    facet: "MIN_RATING",
    label: "Rating",
    section: "About the title",
    quick: true,
    hint: "The community rating, out of 10. Unrated titles never count as at least anything.",
    operators: [
      { id: "at_least", label: "is at least", match: "ANY", input: "number", encode: trimmed },
      { id: "below", label: "isn’t at least", match: "NONE", input: "number", encode: trimmed },
    ],
  },
  { facet: "CONTENT_RATING", label: "Content rating", section: "About the title", quick: true, operators: ONE },
  { facet: "STUDIO", label: "Studio", section: "About the title", operators: MANY },
  { facet: "MOOD", label: "Mood", section: "About the title", operators: MANY },
  { facet: "COLLECTION", label: "Collection", section: "About the title", operators: MANY },
  {
    facet: "ADDED_WITHIN_DAYS",
    label: "Date added",
    section: "About the title",
    operators: [
      { id: "within", label: "in the last", match: "ANY", input: "days", encode: trimmed },
      { id: "before", label: "not in the last", match: "NONE", input: "days", encode: trimmed },
    ],
  },
  {
    facet: "MATCH_STATUS",
    label: "Metadata match",
    section: "About the title",
    hint: "Unmatched titles are the ones no metadata provider has confirmed yet.",
    operators: [{ ...ANY, label: "is" }],
    fixed: [{ value: "unmatched", label: "Unmatched" }, { value: "matched", label: "Matched" }],
  },

  { facet: "DIRECTOR", label: "Director", section: "People", operators: MANY },
  { facet: "ACTOR", label: "Actor", section: "People", operators: MANY },
  { facet: "WRITER", label: "Writer", section: "People", operators: MANY },
  { facet: "PRODUCER", label: "Producer", section: "People", operators: MANY },

  { facet: "RESOLUTION", label: "Resolution", section: "Picture", quick: true, hint: FILES, operators: MANY, label_of: (value) => RESOLUTIONS[value] ?? value },
  { facet: "DYNAMIC_RANGE", label: "Dynamic range", pill: "HDR", section: "Picture", quick: true, hint: `${FILES} HDR10+ isn’t detected, so those titles show as HDR10.`, operators: MANY, label_of: (value) => RANGES[value] ?? value },
  { facet: "VIDEO_CODEC", label: "Video codec", section: "Picture", hint: FILES, operators: MANY, label_of: (value) => codecName(value) },
  { facet: "EDITION", label: "Edition", section: "Picture", hint: "Named cuts, like a Director’s Cut.", operators: MANY },

  { facet: "AUDIO_LAYOUT", label: "Audio layout", section: "Sound", hint: FILES, operators: MANY, label_of: (value) => LAYOUTS[value] ?? `${value} channels` },
  { facet: "AUDIO_CODEC", label: "Audio codec", section: "Sound", hint: FILES, operators: MANY, label_of: (value) => AUDIO_CODECS[value] ?? value.toUpperCase() },
  { facet: "AUDIO_LANGUAGE", label: "Audio language", section: "Sound", hint: FILES, operators: MANY, label_of: (value) => languageName(value) || value },

  { facet: "SUBTITLE_LANGUAGE", label: "Subtitle language", section: "Subtitles", hint: `${FILES} Subtitle files beside the video count.`, operators: MANY, label_of: (value) => languageName(value) || value },
  { facet: "SUBTITLE_CODEC", label: "Subtitle format", section: "Subtitles", hint: FILES, operators: MANY, label_of: (value) => SUBTITLE_CODECS[value] ?? value.toUpperCase() },
];

export const fieldOf = (facet: string): Field => FIELDS.find((field) => field.facet === facet) ?? FIELDS[0]!;
export const operatorOf = (field: Field, id: string): Operator => field.operators.find((operator) => operator.id === id) ?? field.operators[0]!;

/** A value in words: the server's name if it sent one, else one derived from the code. */
export function valueLabel(field: Field, value: string, known?: string): string {
  const fixed = field.fixed?.find((option) => option.value === value)?.label;
  if (fixed) return fixed;
  if (field.label_of) return field.label_of(value);
  if (known) return known;
  if (field.facet === "YEARS" && /^\d{4}-\d{4}$/.test(value)) return `${value.slice(0, 4)}s`;
  return field.facet === "COLLECTION" || field.facet === "EDITION" ? value : value.split("-").map(titleCaseWord).join(" ");
}
