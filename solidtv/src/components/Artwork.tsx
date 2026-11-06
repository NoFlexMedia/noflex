// SPDX-License-Identifier: AGPL-3.0-only
//
// A work's picture, resolving in from a placeholder that holds its shape. Asks the
// server for what the box needs, not the original: texture memory is the budget on a TV.

import { createMemo, createSignal, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { pickImage, sized, type ArtKind } from "$lib/images.js";
import type { IconName } from "$lib/icons.generated.js";
import type { Image } from "$lib/types.js";
import { color, duration, ease, radius } from "../theme.js";
import Icon from "$components/Icon.js";

const KIND_ICON: Record<string, IconName> = { MOVIE: "film-strip", SERIES: "television", SEASON: "television", EPISODE: "television", ALBUM: "music-notes", TRACK: "music-notes", ARTIST: "music-notes", AUDIO_BOOK: "book-open", PHOTO: "image" };

interface ArtworkProps extends NodeProps {
  work: { kind?: string; images?: Image[] | null };
  kind?: ArtKind;
  width: number;
  height: number;
  rounded?: number;
}

export default function Artwork(props: ArtworkProps) {
  const [loaded, setLoaded] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const url = createMemo(() => pickImage(props.work.images, props.kind ?? "poster"));
  const src = () => {
    const known = url();
    return known ? sized(known, props.width, props.height) : undefined;
  };
  const corner = () => props.rounded ?? radius.m;
  const ICON = 72;
  return (
    <view {...props} width={props.width} height={props.height} color={color.surface2} borderRadius={corner()}>
      <Show when={!src() || failed()}>
        <Icon name={KIND_ICON[props.work.kind ?? ""] ?? "film-strip"} size={ICON} x={(props.width - ICON) / 2} y={(props.height - ICON) / 2} color={color.text3} />
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
    </view>
  );
}
