import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import { extractAll } from "./index";

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events: [],
    functions,
    functionsDelta: {},
  };
}

describe("extractAll (full chain: observe -> aggregate -> emit -> write)", () => {
  it("should produce a paste-ready npcDB.lua containing an npc encountered in the session", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 2 } }] },
    });

    const { npcDB } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
    });

    expect(npcDB).toContain("local npcData = {");
    expect(npcDB).toContain("-- Source trace files: 1");
    // minLevel/maxLevel have no observer yet (see observers/npc/minLevel.ts) so they fall back to their schema default (0).
    expect(npcDB).toContain('[823] = {"Deputy Willem",0,0,0,0,0,nil,nil,0,nil,nil,0,nil,nil,0},');
  });

  it("should still emit a schema-complete row (defaults) for fields with no observer yet", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 2 } }] },
    });

    const { npcDB } = extractAll([session], { sourceFileNames: ["test.lua"] });

    const row = npcDB.match(/\[823] = \{([^}]*)},/)?.[1];
    expect(row).toBeDefined();
    // npcKeys has 15 fields; only "name" is extracted so far, rest default.
    expect(row!.split(",")).toHaveLength(15);
  });
});
