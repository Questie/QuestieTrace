import { describe, expect, it } from "vitest";
import { event, makeEpisode, npc } from "../../core/fixtures";
import { EpisodeStates } from "./states";

describe("EpisodeStates", () => {
  const episode = makeEpisode({
    completed: {
      source: "modern",
      initial: [1],
      // 11 is turned in at 50.0; Forever also completes its zone copy 12, which only the
      // delayed completed-set delta reports.
      changes: [{ t: 50.8, add: [11, 12] }],
    },
    questLog: [
      { t: 0, v: [11] },
      { t: 50.9, v: [] },
      { t: 30.9, v: [11, 21] },
    ].sort((a, b) => a.t - b.t),
    questEvents: [event(30, "accepted", 21, npc(5)), event(50, "turnedIn", 11, npc(5)), event(70, "abandoned", 21)],
  });
  const states = new EpisodeStates(episode, { stateSlack: 2, catchUpBatch: 60 });

  it("counts completions the delayed delta reports within the slack, except later turn-ins", () => {
    expect(states.completed(50.1).has(12)).toBe(true);
    expect(states.completed(50.1).has(11)).toBe(true);
    // Read just before the turn-in: 11 is still in progress even though the delta follows soon.
    expect(states.completed(49.5).has(11)).toBe(false);
  });

  it("treats a catch-up read of old completions as completed from the start, except quests active this session", () => {
    const old = Array.from({ length: 70 }, (_, i) => 1000 + i);
    const catchUp = makeEpisode({
      completed: { source: "modern", initial: [], changes: [{ t: 400, add: [...old, 77] }] },
      questEvents: [event(399, "turnedIn", 77, npc(5))],
    });
    const caughtUp = new EpisodeStates(catchUp, { stateSlack: 2, catchUpBatch: 60 });

    expect(caughtUp.isCompleted(1000, 5)).toBe(true);
    expect(caughtUp.isCompleted(77, 5)).toBe(false);
    expect(caughtUp.isCompleted(77, 399)).toBe(true);
  });

  it("patches the sampled log with accepts and removals the sample has not caught up with", () => {
    expect(states.inLog(21, 30.2)).toBe(true);
    expect(states.log(30.2).has(21)).toBe(true);
    expect(states.inLog(21, 70.5)).toBe(false);
    expect(states.inLog(21, 29)).toBe(false);
  });
});
