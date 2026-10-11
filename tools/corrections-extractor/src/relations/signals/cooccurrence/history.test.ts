import { describe, expect, it } from "vitest";
import { event, makeEpisode } from "../../core/fixtures";
import { catalogOf, session } from "./fixtures";
import { buildHistories, order, unstableQuests, type Stamp } from "./history";

const at = (pos: number, exact = false): Stamp => ({ pos, atStart: false, exact, episode: "e", t: pos });

describe("order", () => {
  it("counts any lead of the completion as before, but a short lag of a sampled one as unordered", () => {
    // Completed-quest deltas arrive late, never early.
    expect(order(at(100), at(100.5), 10)).toBe("before");
    expect(order(at(101), at(100), 10)).toBe("unordered");
    expect(order(at(130), at(100), 10)).toBe("after");
    // A turn-in two seconds after accepting the follow-up is no sampling delay.
    expect(order(at(102, true), at(100, true), 10)).toBe("after");
  });
});

describe("buildHistories", () => {
  const options = { ignore: new Set<number>(), maxInitialLoss: 0.1 };

  it("orders a character's sessions by sessionOrder, not by input order", () => {
    const later = session("c", { order: 1, initial: [1, 2], turnIns: [[50, 3]] });
    const earlier = session("c", { order: 0, initial: [1], accepts: [[10, 2]], turnIns: [[20, 2]] });
    const [history] = buildHistories([later, earlier], options).histories;

    expect(history.episodes).toEqual([earlier.key, later.key]);
    expect(order(history.done.get(2)!, history.taken.get(3)!, 10)).toBe("before");
  });

  it("dates quests that appear in a late-loading log to the session start", () => {
    // The log is empty until t=5; quest 3 was never accepted this session, so it was already held.
    const lateLog = makeEpisode({
      characterKey: "c",
      completed: { source: "modern", initial: [1], changes: [{ t: 2.5, add: [2] }] },
      questLog: [
        { t: 0, v: [] },
        { t: 5, v: [3] },
      ],
      questEvents: [event(2, "turnedIn", 2)],
    });
    const [history] = buildHistories([lateLog], options).histories;

    expect(order(history.done.get(2)!, history.taken.get(3)!, 10)).toBe("after");
  });

  it("drops sessions whose completed set was never read", () => {
    // Legacy traces: empty set all session long, although the character turned quests in.
    const broken = makeEpisode({ characterKey: "c", levels: [{ t: 0, v: 20 }], questLog: [{ t: 0, v: [7] }] });
    const { histories, report } = buildHistories([broken], options);

    expect(report.withoutCompleted).toBe(1);
    expect(histories).toEqual([]);
  });
});

describe("unstableQuests", () => {
  it("treats a handful of removals as resets but a dropped-out completed set as a bad read", () => {
    const reset = makeEpisode({ completed: { source: "modern", initial: [1, 2, 3, 4, 5], changes: [{ t: 10, remove: [5] }] } });
    const badRead = makeEpisode({
      completed: { source: "modern", initial: [1, 2, 3, 4, 6], changes: [{ t: 10, remove: [1, 2, 3, 4, 6] }, { t: 11, add: [1, 2, 3, 4, 6] }] },
    });
    expect([...unstableQuests([reset, badRead], catalogOf([]))]).toEqual([5]);
  });
});
