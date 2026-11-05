// SPDX-License-Identifier: AGPL-3.0-only
//
// Grid filters live in the URL, so loaders and the grid read one source. Rules
// are a tree of conditions, and what the server receives comes from it here.
// Viewer-relative conditions don't name the viewer, so a filter means the same
// thing on a linked server.

import { CategoryWorksStore } from "$houdini";
import { fieldOf, operatorOf, type MatchMode } from "$lib/filterFields.js";

// One store per category, so changing a filter keeps the current grid on
// screen (dimmed) while the next one loads.
const categoryStores = new Map<string, CategoryWorksStore>();

export function categoryWorks(slug: string): CategoryWorksStore {
  let store = categoryStores.get(slug);
  if (!store) categoryStores.set(slug, (store = new CategoryWorksStore()));
  return store;
}

export interface RuleNode { kind: "rule"; key: number; facet: string; op: string; inputs: string[] }
export interface GroupNode { kind: "group"; key: number; match: MatchMode; items: TreeNode[] }
export type TreeNode = RuleNode | GroupNode;

let nextKey = 1;
export const newRule = (facet: string, op = fieldOf(facet).operators[0]!.id, inputs: string[] = []): RuleNode => ({ kind: "rule", key: nextKey++, facet, op, inputs });
export const newGroup = (match: MatchMode = "ALL", items: TreeNode[] = []): GroupNode => ({ kind: "group", key: nextKey++, match, items });

/** A rule's values for the server: what's been entered, once it's complete. */
export function ruleValues(rule: RuleNode): string[] {
  const operator = operatorOf(fieldOf(rule.facet), rule.op);
  return operator.encode ? operator.encode(rule.inputs) : rule.inputs;
}

/** `group` without conditions that don't say anything yet, or null if that's all of it. */
export function pruned(group: GroupNode): GroupNode | null {
  const items = group.items.flatMap((item): TreeNode[] => {
    if (item.kind === "rule") return ruleValues(item).length ? [item] : [];
    const kept = pruned(item);
    return kept ? [kept] : [];
  });
  return items.length ? { ...group, items } : null;
}

type Stored = { m: MatchMode; i: StoredItem[] };
type StoredItem = Stored | { f: string; o: string; v: string[] };

const toStored = (group: GroupNode): Stored => ({
  m: group.match,
  i: group.items.map((item) => (item.kind === "group" ? toStored(item) : { f: item.facet, o: item.op, v: item.inputs })),
});

const fromStored = (stored: Stored): GroupNode => newGroup(
  stored.m,
  stored.i.map((item) => ("i" in item ? fromStored(item) : newRule(item.f, item.o, item.v))),
);

/** What goes in the URL; empty when `tree` says nothing. */
export function serializeRules(tree: GroupNode): string {
  const kept = pruned(tree);
  return kept ? JSON.stringify(toStored(kept)) : "";
}

/** The tree in `text`. Anything unreadable, like a hand-edited URL, is no filter at all. */
export function parseRules(text: string | null): GroupNode {
  if (text) {
    try {
      const stored = JSON.parse(text);
      if (stored && Array.isArray(stored.i)) return fromStored(stored);
    } catch { /* falls through to no filter */ }
  }
  return newGroup();
}

function groupInput(group: GroupNode): Record<string, unknown> {
  return {
    match: group.match,
    rules: group.items.flatMap((item) => (item.kind === "rule" ? [{ facet: item.facet, match: operatorOf(fieldOf(item.facet), item.op).match, values: ruleValues(item) }] : [])),
    groups: group.items.flatMap((item) => (item.kind === "group" ? [groupInput(item)] : [])),
  };
}

/** `works` arguments for a grid's URL parameters. */
export function worksVariables(params: URLSearchParams) {
  const filter: Record<string, unknown> = { recursive: params.get("all") === "1" };
  const kind = params.get("kind");
  if (kind) { filter.kinds = [kind]; filter.recursive = true; }
  const rules = pruned(parseRules(params.get("rules")));
  if (rules) filter.rules = groupInput(rules);
  return { filter, sort: (params.get("sort") ?? "DATE_ADDED_DESC") as never };
}
