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
  it("should produce a paste-ready ForeverTraceNpcFixes.lua correction for an npc encountered in the session", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 2 } }] },
    });

    const { npcFixes } = extractAll([session], {
      sourceFileNames: ["test.lua"],
      now: new Date("2020-01-01T00:00:00.000Z"),
    });

    expect(npcFixes).toContain("function ForeverTraceNpcFixes:Load()");
    expect(npcFixes).toContain("-- Source trace files: 1");
    expect(npcFixes).toContain("[823] = {");
    expect(npcFixes).toContain('[npcKeys.name] = "Deputy Willem",');
  });

  it("should only emit fields that were actually observed, not every schema field", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 3, tp: 3, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 3, tp: 3, v: { 1: "Deputy Willem", n: 2 } }] },
    });

    const { npcFixes } = extractAll([session], { sourceFileNames: ["test.lua"] });

    // minLevel/maxLevel have no observer yet (see observers/npc/minLevel.ts), so
    // they must not appear in the correction at all - no defaulting to 0/nil.
    expect(npcFixes).not.toContain("npcKeys.minLevel");
    expect(npcFixes).not.toContain("npcKeys.maxLevel");
  });
});
