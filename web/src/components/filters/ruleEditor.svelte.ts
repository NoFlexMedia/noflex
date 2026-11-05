// SPDX-License-Identifier: AGPL-3.0-only
//
// The filter being edited. It holds conditions still being filled in, which the
// URL leaves out, and tells `commit` whenever what the filter means may have changed.

import { fieldOf, operatorOf } from "$lib/filterFields.js";
import { newGroup, newRule, parseRules, serializeRules, type GroupNode, type RuleNode, type TreeNode } from "$lib/workFilters.js";

export class RuleEditor {
  tree = $state<GroupNode>(newGroup());
  /** The serialized tree last sent to or read from the URL. */
  #committed = "";
  #commit: (serialized: string) => void;

  constructor(commit: (serialized: string) => void) {
    this.#commit = commit;
  }

  /** Follow the URL, unless it only says what this already means. */
  sync(serialized: string): void {
    if (serialized === this.#committed || serialized === serializeRules(this.tree)) { this.#committed = serialized; return; }
    this.#committed = serialized;
    this.tree = parseRules(serialized);
  }

  #changed(): void {
    const serialized = serializeRules(this.tree);
    if (serialized === this.#committed) return;
    this.#committed = serialized;
    this.#commit(serialized);
  }

  replace(serialized: string): void {
    this.tree = parseRules(serialized);
    this.#changed();
  }

  clear(): void { this.replace(""); }

  addRule(group: GroupNode): void {
    group.items.push(newRule("GENRE"));
  }

  addGroup(group: GroupNode): void {
    group.items.push(newGroup("ANY", [newRule("GENRE")]));
  }

  remove(node: TreeNode, from: GroupNode = this.tree): boolean {
    const index = from.items.findIndex((item) => item.key === node.key);
    if (index >= 0) {
      from.items.splice(index, 1);
      this.#changed();
      return true;
    }
    return from.items.some((item) => item.kind === "group" && this.remove(node, item));
  }

  setMatch(group: GroupNode, match: GroupNode["match"]): void {
    group.match = match;
    this.#changed();
  }

  setField(rule: RuleNode, facet: string): void {
    rule.facet = facet;
    rule.op = fieldOf(facet).operators[0]!.id;
    rule.inputs = [];
    this.#changed();
  }

  setOperator(rule: RuleNode, op: string): void {
    const before = operatorOf(fieldOf(rule.facet), rule.op).input;
    rule.op = op;
    if (operatorOf(fieldOf(rule.facet), op).input !== before) rule.inputs = [];
    this.#changed();
  }

  setInputs(rule: RuleNode, inputs: string[]): void {
    rule.inputs = inputs;
    this.#changed();
  }
}
