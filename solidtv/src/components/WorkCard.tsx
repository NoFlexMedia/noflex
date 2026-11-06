// SPDX-License-Identifier: AGPL-3.0-only
//
// A poster or landscape card. Focus does what hover and a press do on the web: the
// card lifts on a spring, so the answer to a key press is immediate.

import { useNavigate } from "@solidjs/router";
import { createSignal, onCleanup, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { titleCase } from "$lib/format.js";
import { WORK_DETAIL } from "$lib/graphql/documents.js";
import { preload } from "$lib/graphql.js";
import { subtitleFor } from "$lib/playback.js";
import type { Work } from "$lib/types.js";
import { ui } from "$lib/ui.js";
import { pickImage } from "$lib/images.js";
import { revealInRow } from "$lib/rows.js";
import { userDataOf } from "$lib/watch.js";
import { color, gradient, motion, radius, toward, type, weight } from "../theme.js";
import Artwork from "$components/Artwork.js";
import Icon from "$components/Icon.js";

export const POSTER = { width: 240, height: 360, total: 460 } as const;
export const WIDE = { width: 448, height: 252, total: 252 } as const;

// Long enough to not fire on every card a held key passes.
const INTENT_MS = 500;
const RING = 6;

export default function WorkCard(props: NodeProps & { work: Work; wide?: boolean; onChosen?: (work: Work) => void }) {
  const navigate = useNavigate();
  const [focused, setFocused] = createSignal(false);
  const size = () => (props.wide ? WIDE : POSTER);
  const data = () => userDataOf(props.work);
  const progress = () => {
    const known = data();
    if (!known || known.played || !known.positionMs || !props.work.runtimeMs) return null;
    return Math.min(1, known.positionMs / props.work.runtimeMs);
  };
  // Where a list asks for the series, an episode leads with it: "Low Water" means nothing alone.
  const heading = () => (props.work.kind === "EPISODE" ? props.work.seriesTitle ?? props.work.title : props.work.title);
  const line = () => {
    const work = props.work;
    const episodeTitle = work.kind === "EPISODE" && work.seriesTitle && !/^episode \d+$/i.test(work.title) ? work.title : "";
    return [subtitleFor(work), episodeTitle, work.kind !== "EPISODE" ? titleCase(work.kind) : ""].filter(Boolean).join(" · ");
  };

  let node: ElementNode | undefined;
  let intent: ReturnType<typeof setTimeout> | undefined;
  let revealing: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => { clearTimeout(intent); clearTimeout(revealing); });
  function onFocusChanged(has: boolean) {
    setFocused(has);
    clearTimeout(intent);
    clearTimeout(revealing);
    if (!has) return;
    revealing = setTimeout(() => { if (node) revealInRow(node); }, 0);
    intent = setTimeout(() => {
      preload(WORK_DETAIL, { id: props.work.id });
      ui.backdrop = pickImage(props.work.images, "backdrop");
    }, INTENT_MS);
  }

  return (
    <view
      {...props}
      ref={(element: ElementNode) => { node = element; }}
      width={size().width}
      height={size().total}
      scale={focused() ? 1.07 : 1}
      zIndex={focused() ? 5 : 1}
      transition={{ scale: motion.spring }}
      onFocusChanged={onFocusChanged}
      onEnter={() => {
        if (props.onChosen) props.onChosen(props.work);
        else navigate(`/works/${props.work.id}`);
        return true;
      }}
    >
      <Artwork work={props.work} kind={props.wide ? "thumb" : "poster"} width={size().width} height={size().height} />
      <view
        x={-RING}
        y={-RING}
        width={size().width + RING * 2}
        height={size().height + RING * 2}
        color={color.clear}
        borderRadius={radius.m + RING}
        border={{ width: 4, color: color.focus }}
        alpha={focused() ? 1 : 0}
        transition={{ alpha: motion.fast }}
      />
      <Show when={data()?.played}>
        <view x={12} y={12} width={40} height={40} borderRadius={20} color={color.accent}>
          <Icon name="check" size={24} x={8} y={8} color={color.accentInk} />
        </view>
      </Show>
      <Show when={progress() != null}>
        <view x={14} y={size().height - 20} width={size().width - 28} height={6} borderRadius={3} color={0xffffff38}>
          <view width={(size().width - 28) * (progress() ?? 0)} height={6} borderRadius={3} color={color.accent} />
        </view>
      </Show>
      <Show when={props.wide} fallback={
        <>
          <text y={size().height + 16} width={size().width} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold} color={focused() ? color.text : color.text2}>{heading()}</text>
          <text y={size().height + 50} width={size().width} contain="width" maxLines={1} fontSize={22} color={color.text3}>{line()}</text>
        </>
      }>
        <view y={size().height - 110} width={size().width} height={110} borderRadius={[0, 0, radius.m, radius.m]} linearGradient={gradient([0x00000000, 0x000000d9], toward.down)} />
        <text x={18} y={size().height - 88} width={size().width - 36} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold}>{heading()}</text>
        <text x={18} y={size().height - 54} width={size().width - 36} contain="width" maxLines={1} fontSize={22} color={color.text2}>{line()}</text>
      </Show>
    </view>
  );
}
