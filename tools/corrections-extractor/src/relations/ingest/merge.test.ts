import { describe, expect, it } from "vitest";
import { SessionMerger } from "./merge";
import { emptyDistillStats, type DistilledSession } from "./types";

function copy(eventCount: number, saved: boolean): DistilledSession {
  return {
    key: "session-1",
    eventCount,
    saved,
    episode: {
      key: "session-1",
      ...(saved ? { sessionName: "2026-10-01_12-00-00" } : {}),
      player: {},
      duration: eventCount,
      levels: [],
      questLog: [],
      questEvents: [],
      offers: [],
    },
    pendingGreetings: [],
    titles: [],
    stats: emptyDistillStats(),
  };
}

const submission = (submissionId: string, contributorId: string, receivedAt: string) => ({ submissionId, contributorId, receivedAt });

describe("SessionMerger", () => {
  it("keeps the most complete copy of a session and remembers every submission it came in", () => {
    const merger = new SessionMerger();
    merger.add(submission("b", "late-contributor", "2026-10-02"), copy(80, false));
    merger.add(submission("a", "first-contributor", "2026-10-01"), copy(50, true));
    merger.add(submission("c", "late-contributor", "2026-10-03"), copy(120, false));

    const merged = merger.merged.get("session-1")!;
    expect(merger.copies).toBe(3);
    expect(merged.session.eventCount).toBe(120);
    // The name only exists on the saved copy; the bigger in-progress copy still gets it.
    expect(merged.session.episode.sessionName).toBe("2026-10-01_12-00-00");
    expect([...merged.submissionIds].sort()).toEqual(["a", "b", "c"]);
    expect(merged.contributorId).toBe("first-contributor");
  });

  it("prefers the saved copy when an in-progress copy has as many events", () => {
    const merger = new SessionMerger();
    merger.add(submission("a", "c", "2026-10-01"), copy(80, false));
    merger.add(submission("b", "c", "2026-10-02"), copy(80, true));
    expect(merger.merged.get("session-1")!.session.saved).toBe(true);
  });
});
