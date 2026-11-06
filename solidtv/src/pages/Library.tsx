// SPDX-License-Identifier: AGPL-3.0-only

import { useParams } from "@solidjs/router";
import { Show } from "solid-js";
import { LIBRARY_WORKS } from "$lib/graphql/documents.js";
import { createPagedWorks } from "$lib/pages.js";
import { app } from "$lib/session.js";
import { PAGE_WIDTH } from "../theme.js";
import Page from "$components/Page.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";
import WorkGrid from "$components/WorkGrid.js";

export default function Library() {
  const params = useParams();
  const paged = createPagedWorks(LIBRARY_WORKS, () => ({ library: params.id }));
  const name = () => app.libraries.find((library) => library.id === params.id)?.name ?? "Library";
  return (
    <Page title={name()}>
      <Show when={paged.items().length} fallback={
        <Show when={paged.error()} fallback={<Show when={paged.fetching()} fallback={<State areaWidth={PAGE_WIDTH} autofocus title="This library is empty" message="Run a scan from a browser once media is in place." />}><Skeleton y={0} /></Show>}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="This library is out of reach" message={paged.error()} action={() => void paged.retry()} />
        </Show>
      }>
        <WorkGrid autofocus items={paged.items()} onEndReached={paged.loadMore} />
      </Show>
    </Page>
  );
}
