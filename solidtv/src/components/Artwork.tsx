// SPDX-License-Identifier: AGPL-3.0-only
//
// A work's picture, resolving in from a placeholder that holds its shape. Asks the
// server for what the box needs, not the original: texture memory is the budget on a TV.

import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { pickImage, sized, type ArtKind } from "$lib/images.js";
import type { IconName } from "$lib/icons.generated.js";
import { INDEXING_LABEL, indexingPulse, latest, watchIndexing } from "$lib/indexing.js";
import type { Image, IndexingStage } from "$lib/types.js";
import { color, duration, ease, radius, weight } from "../theme.js";
import Icon from "$components/Icon.js";

const PULSE = { alpha: { duration: 1000, easing: "ease-in-out" } };
const pulseAlpha = () => (indexingPulse() ? 0.4 : 1);

// In place of the art until there is some. A wide card's title sits along its bottom,
// so there it keeps to the top corner.
function IndexingPlaceholder(props: { stage: IndexingStage; width: number; height: number; wide: boolean }) {
  const circle = () => (props.wide ? 72 : 96);
  const icon = () => circle() / 2;
  const x = () => (props.wide ? 24 : (props.width - circle()) / 2);
  const y = () => (props.wide ? 24 : (props.height - circle() - 44) / 2);
  return (
    <>
      <view x={x()} y={y()} width={circle()} height={circle()} borderRadius={circle() / 2} color={color.surface3} alpha={pulseAlpha()} transition={PULSE} />
      <Icon name="tag" size={icon()} x={x() + icon() / 2} y={y() + icon() / 2} color={color.text} />
      <Show when={props.wide} fallback={
        <text y={y() + circle() + 14} width={props.width} contain="width" maxLines={1} textAlign="center" fontSize={22} fontWeight={weight.semibold} color={color.text2}>{INDEXING_LABEL[props.stage]}</text>
      }>
        <text x={x() + circle() + 16} y={y()} height={circle()} lineHeight={circle()} fontSize={22} fontWeight={weight.semibold} color={color.text2}>{INDEXING_LABEL[props.stage]}</text>
      </Show>
    </>
  );
}

// Art it already has, from beside the media, stays; this says it isn't settled.
function IndexingPill(props: { stage: IndexingStage }) {
  const [labelWidth, setLabelWidth] = createSignal(150);
  return (
    <view x={12} y={12} width={labelWidth() + 62} height={44} borderRadius={22} color={0x000000b8}>
      <Icon name="tag" size={28} x={12} y={8} color={color.text} alpha={pulseAlpha()} transition={PULSE} />
      <text x={48} height={44} lineHeight={44} fontSize={22} fontWeight={weight.semibold} onEvent={{ loaded: (_node, event) => { if (event) setLabelWidth(event.dimensions.w); } }}>{INDEXING_LABEL[props.stage]}</text>
    </view>
  );
}

const KIND_ICON: Record<string, IconName> = { MOVIE: "film-strip", SERIES: "television", SEASON: "television", EPISODE: "television", ALBUM: "music-notes", TRACK: "music-notes", ARTIST: "music-notes", AUDIO_BOOK: "book-open", PHOTO: "image" };

interface ArtworkProps extends NodeProps {
  work: { id?: string; kind?: string; images?: Image[] | null; indexing?: IndexingStage | null };
  kind?: ArtKind;
  width: number;
  height: number;
  rounded?: number;
}

export default function Artwork(props: ArtworkProps) {
  const [loaded, setLoaded] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const work = createMemo(() => latest(props.work));
  const url = createMemo(() => pickImage(work().images, props.kind ?? "poster"));
  // Backdrops and logos sit behind or beside a poster that already says so.
  const stage = createMemo(() => ((props.kind ?? "poster") === "poster" || props.kind === "thumb" ? work().indexing ?? null : null));
  createEffect(() => {
    const id = work().id;
    if (stage() && id) onCleanup(watchIndexing(id));
  });
  const src = () => {
    const known = url();
    return known ? sized(known, props.width, props.height) : undefined;
  };
  const corner = () => props.rounded ?? radius.m;
  const ICON = 72;
  return (
    <view {...props} width={props.width} height={props.height} color={color.surface2} borderRadius={corner()}>
      <Show when={!src() || failed()}>
        <Show when={stage()} fallback={<Icon name={KIND_ICON[work().kind ?? ""] ?? "film-strip"} size={ICON} x={(props.width - ICON) / 2} y={(props.height - ICON) / 2} color={color.text3} />}>
          {(known) => <IndexingPlaceholder stage={known()} width={props.width} height={props.height} wide={props.kind === "thumb"} />}
        </Show>
      </Show>
      <Show when={src()}>
        <view
          src={src()}
          width={props.width}
          height={props.height}
          borderRadius={corner()}
          // The renderer only loads what it would draw, so this can't start at 0.
          alpha={loaded() ? 1 : 0.001}
          transition={{ alpha: { duration: duration.slow, easing: ease.out } }}
          onEvent={{ loaded: () => setLoaded(true), failed: () => setFailed(true) }}
        />
      </Show>
      <Show when={src() && !failed() ? stage() : null}>
        {(known) => <IndexingPill stage={known()} />}
      </Show>
    </view>
  );
}
