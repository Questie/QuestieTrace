import { describe, expect, it } from "vitest";
import { event, makeEpisode } from "./fixtures";
import { EpisodeTimeline } from "./timeline";

describe("EpisodeTimeline", () => {
  const episode = makeEpisode({
    levels: [
      { t: 0, v: 10 },
      { t: 500, v: 11 },
    ],
    completed: {
      source: "modern",
      initial: [1, 2],
      changes: [
        { t: 100, add: [3] },
        { t: 900, remove: [2] },
      ],
    },
    questLog: [
      { t: 0, v: [4] },
      { t: 300, v: [4, 5] },
      { t: 400, v: [5] },
    ],
    questEvents: [event(395, "turnedIn", 4)],
  });
  const timeline = new EpisodeTimeline(episode);

  it("answers completion at a point in time from the initial set, deltas and removals", () => {
    expect(timeline.isCompletedAt(1, 0)).toBe(true);
    expect(timeline.isCompletedAt(3, 99)).toBe(false);
    expect(timeline.isCompletedAt(3, 100)).toBe(true);
    expect(timeline.isCompletedAt(2, 899)).toBe(true);
    expect(timeline.isCompletedAt(2, 900)).toBe(false);
  });

  it("counts a turn-in as completion before the delayed completed-quest delta arrives", () => {
    expect(timeline.isCompletedAt(4, 394)).toBe(false);
    expect(timeline.isCompletedAt(4, 395)).toBe(true);
    expect(new EpisodeTimeline(episode, { includeTurnIns: false }).isCompletedAt(4, 395)).toBe(false);
  });

  it("materializes the completed set and first completion times", () => {
    expect([...timeline.completedAt(1000)].sort()).toEqual([1, 3, 4]);
    expect(timeline.completionTime(1)).toBe("initial");
    expect(timeline.completionTime(3)).toBe(100);
    expect(timeline.completionTime(99)).toBeUndefined();
    expect(timeline.completionsDuringSession()).toEqual([
      { t: 100, v: 3 },
      { t: 395, v: 4 },
    ]);
  });

  it("answers quest log membership and level at a point in time", () => {
    expect(timeline.isInLogAt(5, 299)).toBe(false);
    expect(timeline.isInLogAt(5, 300)).toBe(true);
    expect(timeline.isInLogAt(4, 400)).toBe(false);
    expect(timeline.levelAt(499)).toBe(10);
    expect(timeline.levelAt(500)).toBe(11);
  });
});
