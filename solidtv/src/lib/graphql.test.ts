// SPDX-License-Identifier: AGPL-3.0-only

import { readFileSync } from "node:fs";
import { buildSchema, parse, validate } from "graphql";
import { describe, expect, it } from "vitest";
import * as documents from "$lib/graphql/documents.js";

const schema = buildSchema(readFileSync(new URL("../../../backend/crates/noflex-graphql/tests/snapshots/schema.graphql", import.meta.url), "utf8"));

describe("documents", () => {
  for (const [name, document] of Object.entries(documents)) {
    it(`${name} is valid against the server's schema`, () => {
      expect(validate(schema, parse(document)).map((error) => error.message)).toEqual([]);
    });
  }
});
