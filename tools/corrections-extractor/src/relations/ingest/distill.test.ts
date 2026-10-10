import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../core/types";
import { distillSession } from "./distill";

const creature = (id: number) => `Creature-0-1-2-3-${id}-0000AB`;
const gossipQuest = (questID: number, title = `Quest ${questID}`) => ({ questID, title, isTrivial: false, frequency: 0, repeatable: false });

/**
 * Builds a raw session the way the addon records one: everything inside one frame shares `t`,
 * while `tp` increases with every event and sample, and samples follow the event they react to.
 */
class TraceBuilder {
  private frameT = 0;
  private tp = 0;
  readonly session: SessionRecord = {
    schemaVersion: 12,
    startedAt: 1000,
    startedAtPrecise: 2000.5,
    events: [],
    functions: {
      UnitRace: { player: [{ t: 0, tp: 0, v: { 1: "Orc", 2: "Orc", 3: 2, n: 3 } }] },
      UnitClass: { player: [{ t: 0, tp: 0, v: { 1: "Warrior", 2: "WARRIOR", 3: 1, n: 3 } }] },
      UnitFactionGroup: { player: [{ t: 0, tp: 0, v: { 1: "Horde", 2: "Horde", n: 2 } }] },
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
    },
    functionsDelta: {},
  };

  frame(t: number): this {
    this.frameT = t;
    this.tp = Math.max(this.tp, t);
    return this;
  }

  event(e: string, ...args: unknown[]): this {
    const a: Record<string, unknown> & { n: number } = { n: args.length };
    args.forEach((value, index) => {
      if (value !== undefined) a[index + 1] = value;
    });
    this.session.events.push({ t: this.frameT, tp: this.nextTp(), e, a });
    return this;
  }

  /** `path` is a function name, or [name, param] for parameterized streams. Undefined records a nil read. */
  sample(path: string | [string, string | number], v: unknown, tp = this.nextTp()): this {
    const [name, param] = typeof path === "string" ? [path] : path;
    const functions = this.session.functions as Record<string, any>;
    let stream = functions[name];
    if (param !== undefined) stream = (functions[name] ??= {})[param] ??= [];
    else stream = functions[name] ??= [];
    stream.push(v === undefined ? { t: this.frameT, tp } : { t: this.frameT, tp, v });
    return this;
  }

  /** QuestDialog reads GetQuestID and GetTitleText in one call, so both samples share a `tp`. */
  questDialogRead(questId: number, title: string): this {
    const tp = this.nextTp();
    return this.sample("GetQuestID", questId, tp).sample("GetTitleText", title, tp);
  }

  /** The samples UnitInteraction and QuestDialog take when a dialog opens at `npcId`. */
  dialog(e: string, questId: number, npcId?: number, ...args: unknown[]): this {
    this.event(e, ...args).sample("GetQuestID", questId);
    const guid = npcId === undefined ? undefined : creature(npcId);
    return this.sample(["UnitGUID", "npc"], guid).sample(["UnitGUID", "questnpc"], guid);
  }

  /** QUEST_FINISHED with the closed-state samples. */
  close(): this {
    return this.event("QUEST_FINISHED").sample("GetQuestID", 0).sample(["UnitGUID", "npc"], undefined).sample(["UnitGUID", "questnpc"], undefined);
  }

  gossip(npcId: number, available: ReturnType<typeof gossipQuest>[], active: ReturnType<typeof gossipQuest>[] = []): this {
    return this.event("GOSSIP_SHOW", undefined)
      .sample(["UnitGUID", "npc"], creature(npcId))
      .sample("C_GossipInfo.GetNumAvailableQuests", available.length)
      .sample("C_GossipInfo.GetAvailableQuests", available)
      .sample("C_GossipInfo.GetActiveQuests", active);
  }

  build() {
    return distillSession(this.session);
  }

  private nextTp(): number {
    this.tp = Math.round((this.tp + 0.001) * 1e6) / 1e6;
    return this.tp;
  }
}

const kinds = (result: ReturnType<TraceBuilder["build"]>) => result.episode.questEvents.map((event) => `${event.kind}:${event.questId}`);

describe("distillSession: offers", () => {
  it("turns each GOSSIP_SHOW into a complete snapshot of the giver's lists", () => {
    const { episode } = new TraceBuilder()
      .frame(10)
      .gossip(500, [gossipQuest(1), gossipQuest(2)], [gossipQuest(3)])
      .frame(12)
      .event("GOSSIP_CLOSED", false)
      .sample(["UnitGUID", "npc"], undefined)
      .build();
    expect(episode.offers).toEqual([
      {
        t: expect.any(Number),
        source: "gossip",
        giver: { kind: "npc", id: 500 },
        available: [
          { id: 1, trivial: false, frequency: 0, repeatable: false },
          { id: 2, trivial: false, frequency: 0, repeatable: false },
        ],
        active: [3],
        listComplete: true,
      },
    ]);
  });

  it("does not trust an empty gossip read from a window an addon closed in the same frame", () => {
    const { episode } = new TraceBuilder().frame(10).gossip(500, []).event("GOSSIP_CLOSED", false).build();
    expect(episode.offers[0]).toMatchObject({ available: [], listComplete: false });
  });

  it("does not reuse another giver's non-empty list for a GOSSIP_SHOW that recorded no read of its own", () => {
    const builder = new TraceBuilder().frame(10).gossip(500, [gossipQuest(1)]).frame(11).event("GOSSIP_CLOSED", false).sample(["UnitGUID", "npc"], undefined);
    // Lists are change-only: identical lists and failed reads both leave no sample.
    builder.frame(20).event("GOSSIP_SHOW", undefined).sample(["UnitGUID", "npc"], creature(501));
    builder.frame(21).event("GOSSIP_CLOSED", false).sample(["UnitGUID", "npc"], undefined);
    builder.frame(30).event("GOSSIP_SHOW", undefined).sample(["UnitGUID", "npc"], creature(500));
    const { episode } = builder.build();
    expect(episode.offers.map((offer) => `${offer.giver?.id}:${offer.available.length}:${offer.listComplete}`)).toEqual([
      "500:1:true",
      "501:0:false",
      // The same giver again: an unchanged re-read is the likely story.
      "500:1:true",
    ]);
  });

  it("does not trust a gossip list whose count API disagrees", () => {
    const builder = new TraceBuilder().frame(10).gossip(500, [gossipQuest(1)]);
    builder.sample("C_GossipInfo.GetNumAvailableQuests", 2);
    expect(builder.build().episode.offers[0].listComplete).toBe(false);
  });

  it("reads greeting quest ids per index, falling back to the title where the client gives no id", () => {
    const result = new TraceBuilder()
      .frame(10)
      .event("QUEST_GREETING")
      .sample(["UnitGUID", "npc"], creature(240))
      .sample("GetNumAvailableQuests", 2)
      .sample("GetNumActiveQuests", 1)
      .sample(["GetAvailableQuestInfo", 1], { 1: false, 2: 0, 3: false, 4: false, 5: 147, 6: false, n: 6 })
      .sample(["GetAvailableTitle", 1], "Manhunt")
      // Clients without the quest id return only the first four values.
      .sample(["GetAvailableQuestInfo", 2], { 1: false, 2: 0, 3: false, 4: false, n: 4 })
      .sample(["GetAvailableTitle", 2], "Westbrook Garrison Needs Help!")
      .sample(["GetActiveQuestID", 1], 123)
      .sample(["GetActiveTitle", 1], { 1: "The Collector", 2: true, n: 2 })
      .build();
    expect(result.episode.offers).toEqual([
      {
        t: expect.any(Number),
        source: "greeting",
        giver: { kind: "npc", id: 240 },
        available: [{ id: 147, trivial: false, frequency: 0, repeatable: false }],
        active: [123],
        // Decided once the remaining title is resolved.
        listComplete: false,
      },
    ]);
    expect(result.pendingGreetings).toEqual([{ offer: 0, available: ["Westbrook Garrison Needs Help!"], active: [], expected: 2 }]);
    expect(result.titles).toContainEqual({ locale: "enUS", faction: "Horde", title: "Manhunt", questId: 147, giver: "npc:240" });
  });

  it("treats a detail opened straight from an NPC as that NPC's complete one-quest list", () => {
    const { episode } = new TraceBuilder().frame(10).dialog("QUEST_DETAIL", 7, 500, 0).frame(11).close().event("QUEST_ACCEPTED", 7).build();
    expect(episode.offers).toEqual([{ t: expect.any(Number), source: "detail", giver: { kind: "npc", id: 500 }, available: [{ id: 7 }], listComplete: true }]);
  });

  it("only calls a detail standalone when nothing else explains why it opened", () => {
    const standalone = (setup: (builder: TraceBuilder) => TraceBuilder) =>
      setup(new TraceBuilder()).frame(200).dialog("QUEST_DETAIL", 7, 500, 0).build().episode.offers.at(-1)!.listComplete;

    expect(standalone((b) => b)).toBe(true);
    // The same giver showed a list (without quest 7) recently: a gossip option opened the detail.
    expect(standalone((b) => b.frame(150).gossip(500, [gossipQuest(8)]))).toBe(false);
    // ...but not when that list is older than the window, or another giver was visited since.
    expect(standalone((b) => b.frame(79).gossip(500, [gossipQuest(8)]))).toBe(true);
    expect(standalone((b) => b.frame(150).gossip(500, [gossipQuest(8)]).frame(160).gossip(501, [gossipQuest(9)]))).toBe(true);
    // Another dialog with this giver was open moments before.
    expect(standalone((b) => b.frame(199).dialog("QUEST_PROGRESS", 6, 500))).toBe(false);
    // Pushed by the server right after an accept (the giver's next quest), not opened by a click.
    expect(standalone((b) => b.frame(150).dialog("QUEST_DETAIL", 6, 500, 0).frame(199).close().event("QUEST_ACCEPTED", 6))).toBe(false);
  });

  it("does not trust a list shown while the accept of a quest it lacks was still in flight", () => {
    // The server re-sends the menu before QUEST_ACCEPTED and the quest log update arrive.
    const { episode } = new TraceBuilder().frame(10).gossip(500, [gossipQuest(871)]).frame(10.1).event("QUEST_ACCEPTED", 6541).build();
    expect(episode.offers[0].listComplete).toBe(false);
  });

  it("does not trust an empty gossip page that follows a non-empty one from the same giver (a sub-menu)", () => {
    const { episode } = new TraceBuilder().frame(10).gossip(500, [gossipQuest(1)]).frame(14).gossip(500, []).build();
    expect(episode.offers.map((offer) => offer.listComplete)).toEqual([true, false]);
  });

  it("adds no detail offer for a quest picked from a gossip list; the detail inherits the list's giver", () => {
    const { episode } = new TraceBuilder()
      .frame(10)
      .gossip(500, [gossipQuest(7), gossipQuest(8)])
      .frame(12)
      .event("GOSSIP_CLOSED", false)
      .sample(["UnitGUID", "npc"], undefined)
      // An auto-accept addon closed the detail before questnpc/npc were sampled.
      .event("QUEST_DETAIL", 0)
      .sample("GetQuestID", 7)
      .build();
    expect(episode.offers.map((offer) => offer.source)).toEqual(["gossip"]);
    expect(episode.questEvents).toEqual([{ t: expect.any(Number), kind: "detail", questId: 7, giver: { kind: "npc", id: 500 } }]);
  });

  it("never guesses a giver for an item-started detail", () => {
    // Forever: questStartItemID stays 0 and questnpc holds an Item GUID without the item id,
    // while the NPC targeted moments before must not be taken as the giver.
    const { episode } = new TraceBuilder()
      .frame(9)
      .event("PLAYER_TARGET_CHANGED")
      .sample(["UnitGUID", "target"], creature(600))
      .frame(10)
      .event("QUEST_DETAIL", 0)
      .sample("GetQuestID", 7)
      .sample(["UnitGUID", "questnpc"], "Item-4619-0-400000003542D74B")
      .build();
    expect(episode.questEvents).toEqual([{ t: expect.any(Number), kind: "detail", questId: 7 }]);
    expect(episode.offers).toEqual([{ t: expect.any(Number), source: "detail", available: [{ id: 7 }], listComplete: false }]);
  });

  it("treats a detail right after a turn-in as the finisher's follow-up offer, not a complete list", () => {
    const { episode } = new TraceBuilder()
      .frame(10)
      .dialog("QUEST_COMPLETE", 6, 500)
      .frame(11)
      .event("QUEST_TURNED_IN", 6, 100, 0)
      .frame(11.2)
      .close()
      // The follow-up detail opens in the same frame, before questnpc/npc are readable again.
      .event("QUEST_DETAIL", 0)
      .sample("GetQuestID", 7)
      .build();
    expect(episode.questEvents).toEqual([
      { t: expect.any(Number), kind: "complete", questId: 6, giver: { kind: "npc", id: 500 } },
      { t: expect.any(Number), kind: "turnedIn", questId: 6, giver: { kind: "npc", id: 500 } },
      { t: expect.any(Number), kind: "detail", questId: 7, giver: { kind: "npc", id: 500 } },
    ]);
    expect(episode.offers).toEqual([{ t: expect.any(Number), source: "detail", giver: { kind: "npc", id: 500 }, available: [{ id: 7 }], listComplete: false }]);
  });
});

describe("distillSession: quest ids and givers", () => {
  it("reads the detail's quest id from earlier in the same frame when an addon already closed it", () => {
    // Real ordering seen in traces: the client is already in the new dialog while it dispatches
    // the old dialog's QUEST_FINISHED events, and QUEST_DETAIL arrives after it was auto-accepted.
    const result = new TraceBuilder()
      .frame(10)
      .event("QUEST_FINISHED")
      .questDialogRead(52, "Border Patrol")
      .event("QUEST_FINISHED")
      .questDialogRead(0, "")
      .event("QUEST_DETAIL", 0)
      .frame(10.4)
      .event("QUEST_ACCEPTED", 52)
      .build();
    expect(kinds(result)).toEqual(["detail:52", "accepted:52"]);
    expect(result.titles).toContainEqual({ locale: "enUS", faction: "Horde", title: "Border Patrol", questId: 52 });
  });

  it("never takes the targeted unit as the giver when the dialog's own NPC was not read", () => {
    // A party member's shared quest opens the same kind of detail while some mob is targeted.
    const { episode } = new TraceBuilder()
      .frame(10)
      .event("PLAYER_TARGET_CHANGED")
      .sample(["UnitGUID", "target"], creature(600))
      .frame(11)
      .event("QUEST_DETAIL", 0)
      .sample("GetQuestID", 7)
      .frame(11.5)
      .event("QUEST_ACCEPTED", 7)
      .build();
    expect(episode.questEvents.map((event) => event.giver)).toEqual([undefined, undefined]);
    expect(episode.offers[0].giver).toBeUndefined();
  });

  it("borrows an unread detail's quest id only from the accept that answers its close", () => {
    const detail = (acceptAt: number) =>
      new TraceBuilder()
        .frame(10)
        .event("QUEST_DETAIL", 0)
        .sample("GetQuestID", 0)
        .frame(10.5)
        .event("QUEST_FINISHED")
        .frame(acceptAt)
        .event("QUEST_ACCEPTED", 7)
        .build();
    expect(kinds(detail(10.8))).toEqual(["detail:7", "accepted:7"]);
    // Declined, then something else was accepted a few seconds later.
    expect(kinds(detail(13.6))).toEqual(["accepted:7"]);
  });

  it("carries givers from the detail to QUEST_ACCEPTED and from the turn-in dialog to QUEST_TURNED_IN", () => {
    const { episode } = new TraceBuilder()
      .frame(10)
      .dialog("QUEST_DETAIL", 7, 500, 0)
      .frame(11)
      .close()
      .event("QUEST_ACCEPTED", 3, 7) // Era: (logIndex, questID)
      .frame(50)
      .dialog("QUEST_PROGRESS", 7, 501)
      .frame(51)
      .dialog("QUEST_COMPLETE", 7, 501)
      .frame(52)
      .event("QUEST_TURNED_IN", 7, 100, 0)
      .build();
    expect(episode.questEvents.map((event) => `${event.kind}:${event.questId}@${event.giver?.id}`)).toEqual([
      "detail:7@500",
      "accepted:7@500",
      "progress:7@501",
      "complete:7@501",
      "turnedIn:7@501",
    ]);
  });
});

describe("distillSession: privacy", () => {
  it("never makes a player or a pet the giver, from any unit token", () => {
    // Older traces may still hold player GUIDs; the addon only filters them on newer builds.
    const player = "Player-1234-0053656F";
    const pet = "Pet-0-4615-0-138441-165189-02002A657B";
    const { episode } = new TraceBuilder()
      .frame(10)
      .event("PLAYER_TARGET_CHANGED")
      .sample(["UnitGUID", "target"], player)
      .frame(11)
      .event("GOSSIP_SHOW", undefined)
      .sample(["UnitGUID", "npc"], pet)
      .sample("C_GossipInfo.GetAvailableQuests", [gossipQuest(7)])
      .frame(12)
      .event("QUEST_DETAIL", 0)
      .sample("GetQuestID", 7)
      .sample(["UnitGUID", "questnpc"], player)
      .sample(["UnitGUID", "npc"], player)
      .build();
    expect(JSON.stringify(episode)).not.toMatch(/Player-|Pet-|1234|165189/);
    expect(episode.offers.every((offer) => offer.giver === undefined)).toBe(true);
    expect(episode.questEvents.every((event) => event.giver === undefined)).toBe(true);
  });

  it("keeps only timestamp session names (older builds accepted free text)", () => {
    const named = (name: string) => {
      const builder = new TraceBuilder();
      builder.session.name = name;
      return builder.build().episode.sessionName;
    };
    expect(named("2026-10-01_12-00-00")).toBe("2026-10-01_12-00-00");
    expect(named("raid with Someone")).toBeUndefined();
  });
});

describe("distillSession: client quest flags", () => {
  it("keeps the latest flag values per quest", () => {
    const { episode } = new TraceBuilder()
      .frame(10)
      .sample(["IsBreadcrumbQuest", 92579], false)
      .frame(20)
      .sample(["IsBreadcrumbQuest", 92579], true)
      .sample(["C_QuestLine.GetQuestLineInfo", 92579], { questLineID: 5678, questID: 92579 })
      .sample(["C_QuestInfoSystem.GetQuestClassification", 92580], 6)
      .build();
    expect(episode.questInfo).toEqual({ "92579": { isBreadcrumb: true, questLineId: 5678 }, "92580": { classification: 6 } });
  });
});

describe("distillSession: completed timeline", () => {
  it("collapses an empty completed-quest read (bulk removal, then re-add) but keeps real changes", () => {
    const history = Array.from({ length: 30 }, (_, i) => i + 1);
    const builder = new TraceBuilder();
    builder.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"] = {
      t: 0,
      tp: 0,
      initial: history,
      delta: [
        { t: 5, tp: 5, add: [31] },
        // Loading screen: everything but quest 30 comes back, plus a quest really completed meanwhile.
        { t: 100, tp: 100, remove: [...history, 31] },
        { t: 103, tp: 103, add: [...history.slice(0, 29), 31, 99] },
      ],
    };
    const result = builder.build();
    expect(result.episode.completed!.changes).toEqual([
      { t: 5, add: [31] },
      { t: 100, remove: [30] },
      { t: 103, add: [99] },
    ]);
    expect(result.stats.completedReadGaps).toBe(1);
  });
});

describe("distillSession: removals", () => {
  const removal = (setup: (builder: TraceBuilder) => TraceBuilder) => kinds(setup(new TraceBuilder()).build());

  it("drops the removal that belongs to a turn-in or a completion", () => {
    expect(removal((b) => b.frame(10).event("QUEST_TURNED_IN", 7, 0, 0).frame(10.2).event("QUEST_REMOVED", 7, false))).toEqual(["turnedIn:7"]);
    const completedWithoutTurnInEvent = (b: TraceBuilder) => {
      b.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"] = { t: 0, tp: 0, initial: [], delta: [{ t: 11, tp: 11, add: [7] }] };
      return b.frame(10).event("QUEST_REMOVED", 7, false);
    };
    expect(removal(completedWithoutTurnInEvent)).toEqual([]);
  });

  it("calls a lone removal an abandon, and one next to another quest's accept undecidable", () => {
    expect(removal((b) => b.frame(10).event("QUEST_REMOVED", 7, false))).toEqual(["abandoned:7"]);
    expect(removal((b) => b.frame(10).event("QUEST_ACCEPTED", 8).frame(11).event("QUEST_REMOVED", 7, false))).toEqual(["accepted:8", "removed:7"]);
  });
});
