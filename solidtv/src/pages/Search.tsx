// SPDX-License-Identifier: AGPL-3.0-only

import { Row } from "@solidtv/solid/primitives";
import { createEffect, createSignal, on, Show } from "solid-js";
import { SEARCH } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import type { Work } from "$lib/types.js";
import { color, PAGE_WIDTH, PAGE_X, radius, SCREEN, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";
import Keyboard, { KEYBOARD_WIDTH } from "$components/Keyboard.js";
import State from "$components/State.js";
import { POSTER } from "$components/WorkCard.js";
import WorkGrid from "$components/WorkGrid.js";

const DEBOUNCE_MS = 300;
const RESULTS_X = KEYBOARD_WIDTH + 48;
const RESULTS_WIDTH = PAGE_WIDTH - RESULTS_X;

export default function Search() {
  const [text, setText] = createSignal("");
  const [results, setResults] = createSignal<Work[]>([]);
  const [error, setError] = createSignal("");
  const [searched, setSearched] = createSignal(false);
  let ticket = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(on(text, (value) => {
    clearTimeout(timer);
    const run = ++ticket;
    if (!value.trim()) { setResults([]); setSearched(false); return; }
    timer = setTimeout(async () => {
      try {
        const data = await request(SEARCH, { text: value, mode: "HYBRID" });
        if (run !== ticket) return;
        setResults(data.search.edges.map((edge: { node: { work: Work } }) => edge.node.work).filter(Boolean));
        setError("");
      } catch (cause) {
        if (run === ticket) setError(cause instanceof Error ? cause.message : String(cause));
      }
      if (run === ticket) setSearched(true);
    }, DEBOUNCE_MS);
  }, { defer: true }));

  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={2}>
      <view y={56} width={KEYBOARD_WIDTH} height={88} borderRadius={radius.m} color={color.surface2} skipFocus>
        <Icon name="magnifying-glass" size={36} x={26} y={26} color={color.text3} />
        <text x={80} width={KEYBOARD_WIDTH - 104} height={88} lineHeight={88} contain="both" maxLines={1} fontSize={type.lead} color={text() ? color.text : color.text3}>{text() || "Search titles, people, moods"}</text>
      </view>
      <Show when={text() && searched() && !results().length}>
        <view x={RESULTS_X} width={RESULTS_WIDTH} height={SCREEN.height} skipFocus>
          <State areaWidth={RESULTS_WIDTH} kind={error() ? "error" : "empty"} icon="magnifying-glass" title={error() ? "Search is out of reach" : "Nothing matched"} message={error() || `No titles match “${text()}”.`} />
        </view>
      </Show>
      <Show when={!text()}>
        <text x={RESULTS_X} y={80} fontSize={type.lead} fontWeight={weight.semibold} color={color.text3}>Results show up here as you type.</text>
      </Show>
      <Row autofocus y={180} width={PAGE_WIDTH} height={SCREEN.height - 180} scroll="none" gap={48}>
        <Keyboard autofocus value={text()} onChange={setText} />
        <Show when={results().length}>
          <WorkGrid y={-124} items={results()} columns={Math.floor(RESULTS_WIDTH / (POSTER.width + 36))} />
        </Show>
      </Row>
    </view>
  );
}
