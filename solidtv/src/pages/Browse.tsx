// SPDX-License-Identifier: AGPL-3.0-only

import { useNavigate, useParams } from "@solidjs/router";
import { Column } from "@solidtv/solid/primitives";
import { createMemo, Show } from "solid-js";
import { categoryBySlug } from "$lib/categories.js";
import { CATEGORY_SHELVES, CATEGORY_WORKS } from "$lib/graphql/documents.js";
import { createQuery } from "$lib/graphql.js";
import { createPagedWorks } from "$lib/pages.js";
import type { Connection, Work } from "$lib/types.js";
import { PAGE_WIDTH, SCREEN } from "../theme.js";
import Button from "$components/Button.js";
import Page from "$components/Page.js";
import Rail, { railHeight } from "$components/Rail.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";
import WorkGrid from "$components/WorkGrid.js";

interface Shelves { recommendations: Connection<Work>; newlyReleased: Connection<Work>; recentlyAdded: Connection<Work> }

function ShelvesView(props: { slug: string; category: string; label: string }) {
  const navigate = useNavigate();
  const shelves = createQuery<Shelves>(CATEGORY_SHELVES, () => ({ category: props.category }));
  const data = shelves.data;
  return (
    <Show when={data()} fallback={
      <Show when={shelves.error()} fallback={<Skeleton y={0} />}>
        <State areaWidth={PAGE_WIDTH} autofocus kind="error" title={`${props.label} is out of reach`} message={shelves.error()} action={() => void shelves.refetch()} />
      </Show>
    }>
      <Column autofocus width={PAGE_WIDTH} height={SCREEN.height * 2} scroll="always" gap={8}>
        <Button label={`All ${props.label}`} icon="caret-right" onEnter={() => { navigate(`/browse/${props.slug}/all`); return true; }} />
        <Show when={data()!.recommendations.nodes.length}><Rail title="Recommended" items={data()!.recommendations.nodes} height={railHeight()} /></Show>
        <Show when={data()!.newlyReleased.nodes.length}><Rail title="Newly released" items={data()!.newlyReleased.nodes} height={railHeight()} /></Show>
        <Show when={data()!.recentlyAdded.nodes.length}><Rail title="Recently added" items={data()!.recentlyAdded.nodes} height={railHeight()} /></Show>
      </Column>
    </Show>
  );
}

export function GridView(props: { category: string; label: string }) {
  const paged = createPagedWorks(CATEGORY_WORKS, () => ({ category: props.category }));
  return (
    <Show when={paged.items().length} fallback={
      <Show when={paged.error()} fallback={<Show when={paged.fetching()} fallback={<State areaWidth={PAGE_WIDTH} autofocus title={`No ${props.label.toLowerCase()} yet`} message="Nothing here so far." />}><Skeleton y={0} /></Show>}>
        <State areaWidth={PAGE_WIDTH} autofocus kind="error" title={`${props.label} is out of reach`} message={paged.error()} action={() => void paged.retry()} />
      </Show>
    }>
      <WorkGrid autofocus items={paged.items()} onEndReached={paged.loadMore} />
    </Show>
  );
}

export default function Browse() {
  const params = useParams();
  const category = createMemo(() => categoryBySlug(params.category ?? ""));
  return (
    <Page title={category()?.label ?? "Browse"}>
      <Show when={category()} keyed fallback={<State areaWidth={PAGE_WIDTH} autofocus kind="error" title="There’s nothing here" message="This category doesn’t exist on this server." />}>
        {(known) => (known.shelves
          ? <ShelvesView slug={known.slug} category={known.value} label={known.label} />
          : <GridView category={known.value} label={known.label} />)}
      </Show>
    </Page>
  );
}
