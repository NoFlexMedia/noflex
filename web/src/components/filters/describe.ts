// SPDX-License-Identifier: AGPL-3.0-only
//
// A filter in words, so a tree of conditions can be read back and checked.

import { fieldOf, operatorOf, valueLabel } from "$lib/filterFields.js";
import { pruned, type GroupNode, type RuleNode, type TreeNode } from "$lib/workFilters.js";

type Names = (facet: string, value: string) => string | undefined;

const list = (items: string[], word: string) => (items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} ${word} ${items.at(-1)}`);

function describeRule(rule: RuleNode, names: Names): string {
  const field = fieldOf(rule.facet);
  const operator = operatorOf(field, rule.op);
  const label = field.label;
  const values = rule.inputs.map((value) => valueLabel(field, value, names(rule.facet, value)));
  switch (operator.input) {
    case "none": return `is${operator.match === "NONE" ? "n’t" : ""} a favorite`;
    case "pick":
      if (operator.match === "ALL") return `${label} includes ${list(values, "and")}`;
      return `${label} ${operator.match === "NONE" ? "isn’t" : "is"} ${list(values, "or")}`;
    case "years": return `${label} is between ${rule.inputs[0]} and ${rule.inputs[1]}`;
    case "days": return `${label} ${operator.label} ${rule.inputs[0]} days`;
    case "text": return `${label} ${operator.label} “${rule.inputs[0]}”`;
    default: return `${label} ${operator.label} ${rule.inputs[0]}`;
  }
}

function describeNode(node: TreeNode, names: Names, nested: boolean): string {
  if (node.kind === "rule") return describeRule(node, names);
  const parts = node.items.map((item) => describeNode(item, names, true));
  if (node.match === "NONE") return `none of (${parts.join("; ")})`;
  const joined = parts.join(node.match === "ALL" ? " and " : " or ");
  return nested && parts.length > 1 ? `(${joined})` : joined;
}

/** `tree` as a clause, leaving out conditions not filled in; null when nothing is. */
export function describeTree(tree: GroupNode, names: Names): string | null {
  const kept = pruned(tree);
  return kept && describeNode(kept, names, false);
}
