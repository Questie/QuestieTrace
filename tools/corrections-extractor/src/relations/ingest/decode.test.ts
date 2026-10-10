import { describe, expect, it } from "vitest";
import { decodeExportString } from "./decode";

// A synthetic export (no real trace data) built with the addon's own pipeline: BlizzardCBOR
// SerializeCBOR -> LibDeflate CompressDeflate -> EncodeForPrint, from tools/decoder/dependencies.
// It covers packed args, a nil hole, nil stream values, index-keyed streams, floats and UTF-8.
const FIXTURE =
  "!QuestieTrace:1!9azZoonmuuyNuqiH4bG)eQI9gLMoaBqi8KKMuGP)qAhMDvEsUWejhNOyNkydI2rJepbSA2uAfiXgEdytFrybVcGuHfGDs04fjw375E)o(SoqacrsgxS4lUCAkeABz)aSvhSL9mRUyR9Ww3Vhmh4sXYne6AJRyEtKrYAc3Kadgp1lCYmIJJ3OjEUe5U9q7lZvFvsxr4ge4z(ddd7pAwyWWxsKg6Upg1)vL8iPM6N7fMtleWI1ejIm)0R(7RFhLgK)GY0JHcrt9ZAV77OB8VF8339WF(OpGkp)wO2OQJwCWuEI0FAF3vo88OLQrmiZp62ofavwwaylChSnUlUJDx1ZrDi7x7JprKMQBMvl4f0iyvVCg9Tqrd2ngEbLPuUz9VwgeElnWN6dYXLGq23DzTYkFudUkEQ3)yLoYCAcJEmdMKizWYf1IgU9C(2VfLfdTRQxRVPzVqikJh3KvcjTqcXQW9WGderNaP0dvjJk9U2GlYrxGjPRo6UoZQ81ZZE990SzmNS0CgOMVXVInk)QnRxSEenXlRb59ehAC8Il5NOIIek7udZwJUa9Ocikra)X7JF99haVjpRq24bJXNqfEvv0psxQK(R))d!End:QuestieTrace:1!";

describe("decodeExportString", () => {
  it("produces the shapes loadTraceFile + normalizeLuaValue produce", () => {
    const [trace] = decodeExportString(FIXTURE);
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

  it("decodes every export of a submission that pasted several, even when line-wrapped", () => {
    const wrapped = FIXTURE.replace(/(.{76})/g, "$1\n");
    const traces = decodeExportString(`${FIXTURE}\n${wrapped}`);
    expect(traces).toHaveLength(2);
    expect(traces[1]).toEqual(traces[0]);
  });

  it("fails on missing markers and on characters outside the print alphabet", () => {
    expect(() => decodeExportString("not an export")).toThrow(/markers/);
    expect(() => decodeExportString("!QuestieTrace:1!abc!def!End:QuestieTrace:1!")).toThrow(/print-encoded/);
  });
});
