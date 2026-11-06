// SPDX-License-Identifier: AGPL-3.0-only

import { useNavigate } from "@solidjs/router";
import { Column, Row } from "@solidtv/solid/primitives";
import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { HOME } from "$lib/graphql/documents.js";
import { createQuery } from "$lib/graphql.js";
import { pickImage } from "$lib/images.js";
import { player, playWork } from "$lib/playback.js";
import { runtime, titleCase } from "$lib/format.js";
import { app } from "$lib/session.js";
import type { Work } from "$lib/types.js";
import { poll, ui } from "$lib/ui.js";
import { userDataOf } from "$lib/watch.js";
import { color, PAGE_WIDTH, PAGE_X, SCREEN, type, weight } from "../theme.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";
import Rail, { railHeight } from "$components/Rail.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";

const TOP_LEVEL = ["MOVIE", "SERIES", "ALBUM", "AUDIO_BOOK"];
const DWELL = 9000;
const HERO_HEIGHT = 760;
const STACK_TOP = 120;
const STACK_BOTTOM = 572;

interface HomeData {
  contentCategories: { category: string; workCount: number }[];
  continueWatching: { nodes: Work[] };
  recentlyAdded: { nodes: Work[] };
  recentlyPlayed: { nodes: Work[] };
  recommendations: { nodes: Work[] };
}

export function resumeLabel(work: Work, known = userDataOf(work)): string {
  if (known && !known.played && (known.positionMs ?? 0) > 60_000) {
    const left = work.runtimeMs ? runtime(work.runtimeMs - (known.positionMs ?? 0)) : "";
    return left ? `Resume · ${left} left` : "Resume";
  }
  return "Play";
}

function Hero(props: { items: Work[] }) {
  const navigate = useNavigate();
  const [index, setIndex] = createSignal(0);
  const [holding, setHolding] = createSignal(false);
  const current = () => props.items[index() % Math.max(1, props.items.length)]!;

  // Only rotates while it has the viewer's attention, so an idle TV can idle.
  createEffect(() => {
    if (!holding() || props.items.length < 2) return;
    onCleanup(poll(() => setIndex((value) => (value + 1) % props.items.length), DWELL));
  });
  createEffect(() => { if (holding()) ui.backdrop = pickImage(current().images, "backdrop"); });

  const meta = () => [current().year, titleCase(current().kind), runtime(current().runtimeMs)].filter(Boolean).join("  ·  ");

  return (
    <view width={PAGE_WIDTH} height={HERO_HEIGHT} forwardFocus={0} onFocusChanged={setHolding}>
      <Row y={596} width={1000} height={80} gap={20} scroll="none">
        <Button
          size="lg"
          kind="light"
          icon="play"
          busy={player.preparing === current().id}
          label={resumeLabel(current())}
          onEnter={() => { void playWork(current()); return true; }}
        />
        <Button size="lg" icon="info" label="More info" onEnter={() => { navigate(`/works/${current().id}`); return true; }} />
      </Row>
      <view y={STACK_TOP} width={1200} height={STACK_BOTTOM - STACK_TOP} display="flex" flexDirection="column" justifyContent="flexEnd" gap={18} skipFocus>
        <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text3} letterSpacing={3}>JUST ADDED</text>
        <text width={1200} contain="width" maxLines={2} lineHeight={92} fontSize={type.display} fontWeight={weight.bold}>{current().title}</text>
        <view width={1000} height={36} display="flex" gap={14} alignItems="center">
          <Show when={current().communityRating}>
            <Icon name="star" size={28} color={color.warning} />
            <text fontSize={type.caption} fontWeight={weight.semibold} color={color.warning}>{current().communityRating?.toFixed(1)}</text>
          </Show>
          <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{meta()}</text>
        </view>
        <text width={1000} contain="width" maxLines={3} lineHeight={42} fontSize={type.body} color={color.text2}>{current().overview ?? ""}</text>
      </view>
    </view>
  );
}

export default function Home(props: { isAlive?: () => boolean }) {
  const home = createQuery<HomeData>(HOME);
  const data = home.data;

  // Back from the player: Continue watching has moved on.
  let wasPlaying = false;
  createEffect(() => {
    const playing = !!player.current;
    if (wasPlaying && !playing && props.isAlive?.() !== false) void home.refetch();
    wasPlaying = playing;
  });

  createEffect(() => { const rows = data()?.contentCategories; if (rows) app.categories = rows; });

  const featured = createMemo(() => {
    const pool = (data()?.recentlyAdded.nodes ?? []).filter((work) => TOP_LEVEL.indexOf(work.kind) >= 0);
    const withArt = pool.filter((work) => pickImage(work.images, "backdrop"));
    return (withArt.length ? withArt : pool).slice(0, 6);
  });
  const empty = () => !!data() && !data()!.recentlyAdded.nodes.length && !data()!.continueWatching.nodes.length;

  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={0}>
      <Show when={data()} fallback={
        <Show when={home.error()} fallback={<Skeleton y={140} />}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="Home is out of reach" message={home.error()} action={() => void home.refetch()} />
        </Show>
      }>
        <Show when={!empty()} fallback={
          <State areaWidth={PAGE_WIDTH} autofocus icon="sparkle" title={`Welcome, ${app.viewer?.name ?? ""}.`} message={app.viewer?.isAdmin ? "Your libraries are empty so far. Add one from a browser, and run a scan once media is in place." : "Nothing has been shared with you yet. Ask the server’s administrator to grant you a library."} action={() => void home.refetch()} actionLabel="Check again" />
        }>
          <Column autofocus width={PAGE_WIDTH} height={SCREEN.height * 3} scroll="always" gap={8}>
            <Show when={featured().length}><Hero items={featured()} /></Show>
            <Show when={data()!.continueWatching.nodes.length}><Rail title="Continue watching" items={data()!.continueWatching.nodes} wide height={railHeight(true)} /></Show>
            <Show when={data()!.recentlyAdded.nodes.length}><Rail title="Recently added" items={data()!.recentlyAdded.nodes} height={railHeight()} /></Show>
            <Show when={data()!.recommendations.nodes.length}><Rail title="Recommended" items={data()!.recommendations.nodes} height={railHeight()} /></Show>
            <Show when={data()!.recentlyPlayed.nodes.length}><Rail title="Watch again" items={data()!.recentlyPlayed.nodes} height={railHeight()} /></Show>
          </Column>
        </Show>
      </Show>
    </view>
  );
}

