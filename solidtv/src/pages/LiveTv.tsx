// SPDX-License-Identifier: AGPL-3.0-only
//
// Channels and what's on them. Tune in and watch; the web's programme grid and
// recording rules want a pointer and a keyboard, and stay there.

import { VirtualGrid } from "@solidtv/solid/primitives";
import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { LIVE_CHANNELS, LIVE_GUIDE } from "$lib/graphql/documents.js";
import { createQuery, request } from "$lib/graphql.js";
import { playChannel, player } from "$lib/playback.js";
import { mediaUrl } from "$lib/tokens.js";
import { poll } from "$lib/ui.js";
import { app } from "$lib/session.js";
import { color, motion, PAGE_WIDTH, radius, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";
import Page from "$components/Page.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";

interface Channel { id: string; name: string; number?: string | null; logoUrl?: string | null; group?: string | null; enabled: boolean }
interface Program { channelId: string; title: string; startAt: string; endAt: string }

const CARD = { width: 540, height: 200 };
const GAP = 28;
const MINUTE = 60_000;

function ChannelCard(props: NodeProps & { channel: Channel; program?: Program; now: number }) {
  const [focused, setFocused] = createSignal(false);
  const progress = () => {
    if (!props.program) return 0;
    const start = Date.parse(props.program.startAt);
    return Math.min(1, (props.now - start) / (Date.parse(props.program.endAt) - start));
  };
  const logo = () => mediaUrl(props.channel.logoUrl);
  return (
    <view
      {...props}
      width={CARD.width}
      height={CARD.height}
      borderRadius={radius.l}
      color={focused() ? color.surface3 : color.surface}
      border={{ width: 3, color: focused() ? color.focus : color.line }}
      scale={focused() ? 1.04 : 1}
      zIndex={focused() ? 2 : 1}
      transition={{ scale: motion.spring }}
      onFocusChanged={setFocused}
      onEnter={() => { void playChannel(props.channel); return true; }}
    >
      <Show when={logo()} fallback={<Icon name="television" size={52} x={28} y={28} color={color.text3} />}>
        <view src={logo() ?? undefined} x={24} y={24} width={64} height={64} />
      </Show>
      <text x={112} y={24} width={CARD.width - 136} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold} color={color.accent}>{props.channel.number ?? "Live"}</text>
      <text x={112} y={56} width={CARD.width - 136} contain="width" maxLines={1} fontSize={type.body} fontWeight={weight.semibold}>{props.channel.name}</text>
      <text x={24} y={118} width={CARD.width - 48} contain="width" maxLines={1} fontSize={type.caption} color={color.text2}>{props.program?.title ?? "No guide information"}</text>
      <Show when={props.program}>
        <view x={24} y={166} width={CARD.width - 48} height={6} borderRadius={3} color={color.surface3}>
          <view width={(CARD.width - 48) * progress()} height={6} borderRadius={3} color={color.accent} />
        </view>
      </Show>
    </view>
  );
}

export default function LiveTv() {
  const channels = createQuery<{ liveTvChannels: { nodes: Channel[] } }>(LIVE_CHANNELS);
  const list = createMemo(() => channels.data()?.liveTvChannels.nodes.filter((channel) => channel.enabled) ?? []);
  const [onNow, setOnNow] = createSignal(new Map<string, Program>());
  const [now, setNow] = createSignal(Date.now());

  const loadGuide = async () => {
    const ids = list().map((channel) => channel.id);
    if (!ids.length) return;
    const start = new Date(Date.now()).toISOString();
    const end = new Date(Date.now() + MINUTE).toISOString();
    try {
      const data = await request<{ liveTvGuide: { nodes: Program[] } }>(LIVE_GUIDE, { channelIds: ids, start, end });
      setOnNow(new Map(data.liveTvGuide.nodes.map((program) => [program.channelId, program])));
    } catch { /* cards say there's no guide */ }
  };
  createEffect(() => { void loadGuide(); });
  createEffect(() => {
    onCleanup(poll(() => { if (!player.current) { setNow(Date.now()); void loadGuide(); } }, MINUTE));
  });

  return (
    <Page title="Live TV">
      <Show when={channels.data()} fallback={
        <Show when={channels.error()} fallback={<Skeleton y={0} />}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="Live TV is out of reach" message={channels.error()} action={() => void channels.refetch()} />
        </Show>
      }>
        <Show when={list().length} fallback={<State areaWidth={PAGE_WIDTH} autofocus icon="television" title="No channels yet" message={app.viewer?.isAdmin ? "Add an M3U tuner source from a browser and its channels appear here." : "Live TV hasn’t been set up on this server yet."} />}>
          <VirtualGrid autofocus width={3 * (CARD.width + GAP)} columns={3} rows={4} buffer={2} gap={GAP} scroll="always" each={list()}>
            {(channel) => <ChannelCard channel={channel()} program={onNow().get(channel().id)} now={now()} />}
          </VirtualGrid>
        </Show>
      </Show>
    </Page>
  );
}
