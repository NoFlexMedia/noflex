// SPDX-License-Identifier: AGPL-3.0-only

import { useParams } from "@solidjs/router";
import { Show } from "solid-js";
import { categoryBySlug } from "$lib/categories.js";
import Page from "$components/Page.js";
import { GridView } from "$pages/Browse.js";

export default function BrowseAll() {
  const params = useParams();
  const category = () => categoryBySlug(params.category ?? "");
  return (
    <Page title={`All ${category()?.label ?? ""}`}>
      <Show when={category()} keyed>{(known) => <GridView category={known.value} label={known.label} />}</Show>
    </Page>
  );
}
