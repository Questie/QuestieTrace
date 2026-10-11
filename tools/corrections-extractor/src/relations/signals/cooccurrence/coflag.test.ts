import { describe, expect, it } from "vitest";
import { inferCoFlags } from "./coflag";
import { catalogOf, populationOf, session } from "./fixtures";

const params = { windowSeconds: 3, minObservations: 2 };
const catalog = catalogOf([8, 41, 42]);

describe("inferCoFlags", () => {
  it("pairs copies the server completes together, whichever one was turned in", () => {
    const episodes = [
      ...["a", "b"].map((key) => session(key, { initial: [8], turnIns: [[100, 41]], flags: [[100.5, 42]] })),
      ...["c", "d"].map((key) => session(key, { initial: [8], turnIns: [[100, 42]], flags: [[100.5, 41]] })),
    ];
    const [edge, ...rest] = inferCoFlags(populationOf(episodes, catalog), params).edges;

    expect(rest).toEqual([]);
    expect(edge).toMatchObject({ a: 41, b: 42, observations: 4, turnedInA: 2, turnedInB: 2, contradict: 0 });
  });

  it("rejects a breadcrumb that its target flags, once a character completes the breadcrumb alone", () => {
    // Turning in target 42 flags breadcrumb 41, but character c took 41 itself, long before 42.
    const flagged = ["a", "b"].map((key) => session(key, { initial: [8], turnIns: [[100, 42]], flags: [[100.5, 41]] }));
    const walkedIt = session("c", { initial: [8], turnIns: [[50, 41], [400, 42]] });

    expect(inferCoFlags(populationOf(flagged, catalog), params).edges.map(({ a, b }) => [a, b])).toEqual([[41, 42]]);
    expect(inferCoFlags(populationOf([...flagged, walkedIt], catalog), params).edges).toEqual([]);
  });

  it("covers only quests seen landing in the completed set during a session", () => {
    // 41 lands mid-session for two characters; 42 was always completed before capture started.
    const episodes = ["a", "b"].map((key) => session(key, { initial: [8, 42], turnIns: [[100, 41]] }));

    expect(inferCoFlags(populationOf(episodes, catalog), params).covered).toEqual([41]);
  });

  it("rejects a pair once a character held both at the same time", () => {
    const flagged = ["a", "b"].map((key) => session(key, { initial: [8], turnIns: [[100, 42]], flags: [[100.5, 41]] }));
    const heldBoth = session("c", { initial: [8], accepts: [[10, 41], [20, 42]], turnIns: [[100, 42]], flags: [[100.5, 41]] });

    expect(inferCoFlags(populationOf([...flagged, heldBoth], catalog), params).edges).toEqual([]);
  });
});
