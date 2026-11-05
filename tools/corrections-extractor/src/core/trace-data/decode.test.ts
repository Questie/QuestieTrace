import { describe, expect, it } from "vitest";
import { decodeExportString } from "./decode";
import { TINY_EXPORT } from "./fixtures";

describe("decodeExportString", () => {
  it("should produce arrays, packed args and index-keyed streams like the addon recorded them", () => {
    const [trace] = decodeExportString(TINY_EXPORT).traces;
    const session = trace.sessions[0];
    const functions = session.functions as Record<string, any>;
    // Packed args keep their `n` and stay objects; nil args are simply absent.
    expect(session.events.map((event) => event.a)).toEqual([{ "1": 7, "2": 92579, n: 2 }, { n: 1 }]);
    // A nil stream value leaves no `v` key; a nil hole in an array leaves a sparse object.
    expect(functions.UnitGUID.npc[1]).toEqual({ t: 2, tp: 2 });
    expect(functions.Sparse[0].v).toEqual({ "1": 10, "3": 30 });
    // Index-keyed streams stay maps so getStream(session, name, index) works.
    expect(functions.GetAvailableTitle).toEqual({ "1": [{ t: 1, tp: 1.5, v: "Ünïcode Title" }], "2": [{ t: 1, tp: 1.5, v: "Second" }] });
    expect(functions.Numbers[0].v).toEqual([-1, -1.5, 65504, 1e300, 2 ** 53]);
    expect(session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]).toEqual({ t: 0, tp: 0, initial: [1, 2, 3], delta: [{ t: 5, tp: 5.25, add: [4] }] });
  });

  it("should decode every export of a submission that pasted several, even when line-wrapped", () => {
    const wrapped = TINY_EXPORT.replace(/(.{76})/g, "$1\n");
    const { traces, dropped } = decodeExportString(`${TINY_EXPORT}\n${wrapped}`);
    expect(traces).toHaveLength(2);
    expect(traces[1]).toEqual(traces[0]);
    expect(dropped).toEqual([]);
  });

  it("should decode an export pasted into another one and drop the one it cut off", () => {
    const cutOff = TINY_EXPORT.slice(0, 200);
    const tail = TINY_EXPORT.slice(-60);

    const { traces, dropped } = decodeExportString(`${cutOff}${TINY_EXPORT}${tail}`);

    expect(traces).toEqual(decodeExportString(TINY_EXPORT).traces);
    expect(dropped).toEqual(["export 1 has no matching end marker"]);
  });

  it("should keep the exports that decode when another export of the submission does not", () => {
    const broken = "!QuestieTrace:1!abc$def!End:QuestieTrace:1!";

    const { traces, dropped } = decodeExportString(`${broken}\n${TINY_EXPORT}`);

    expect(traces).toEqual(decodeExportString(TINY_EXPORT).traces);
    expect(dropped).toEqual([expect.stringMatching(/^export 1: payload is not valid print-encoded data/)]);
  });

  it("should fail when no export decodes, on missing markers and on characters outside the print alphabet", () => {
    expect(() => decodeExportString("not an export")).toThrow(/markers/);
    expect(() => decodeExportString("!QuestieTrace:1!abc$def!End:QuestieTrace:1!")).toThrow(/print-encoded/);
    expect(() => decodeExportString(TINY_EXPORT.slice(0, 200))).toThrow(/no matching end marker/);
  });
});
