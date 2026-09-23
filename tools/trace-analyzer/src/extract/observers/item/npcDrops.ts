// itemKeys.npcDrops - Questie field 2.
//
// Reads GetLootSourceInfo(slot) streams (sampled by Modules/Trackers/Loot.lua on
// LOOT_READY/LOOT_CLOSED) and correlates each loot slot's source GUID with the
// item ID from GetLootSlotLink(slot) at the same time.
//
// Each observation represents one piece of evidence: "at time t, player looted
// item X from NPC Y". The custom merge unions NPC IDs per item across all
// sessions.
//
// Privacy: GetLootSourceInfo is already filtered by the Loot tracker - player
// GUIDs and unrecognized kinds are discarded before recording, so we only ever
// see npc/object/item GUIDs here.

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { parseGuid } from "../../guid";
import { parseItemLink } from "../../itemLink";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export const observeNpcDrops: FieldObserver<{ npcID: number }> = (session) => {
  const observations: Observation<{ npcID: number }>[] = [];
  const linkStream = session.functions["GetLootSlotLink"];
  const sourceStream = session.functions["GetLootSourceInfo"];
  if (!linkStream || !sourceStream) return observations;

  // Walk each loot slot that has both a link and a source
  for (const slot of getParamKeys(linkStream)) {
    const linkEntries = getStream(session, "GetLootSlotLink", slot);
    const sourceEntries = getStream(session, "GetLootSourceInfo", slot);
    if (!linkEntries || !sourceEntries) continue;

    // For each point in time both were sampled (same t), correlate
    for (const linkEntry of linkEntries) {
      if (typeof linkEntry.v !== "string") continue;
      const parsedLink = parseItemLink(linkEntry.v);
      if (!parsedLink) continue;

      const itemID = parsedLink.itemID;
      const t = linkEntry.t;

      // Find the source info at the same time
      const sourceAt = sourceEntries.find((e) => e.t === t);
      if (!sourceAt || typeof sourceAt.v !== "object" || !sourceAt.v) continue;

      // GetLootSourceInfo returns packed args: guid1, qty1, guid2, qty2, ...
      // Iterate pairs
      const source = sourceAt.v as { n?: number; [key: number]: unknown };
      for (let i = 1; i <= (source.n ?? 0); i += 2) {
        const guid = source[i];
        if (typeof guid !== "string") continue;
        const parsed = parseGuid(guid);
        if (parsed.kind === "npc" && parsed.id !== null) {
          observations.push({
            entityId: itemID,
            value: { npcID: parsed.id },
            confidence: "high",
            provenance: { session: sessionLabel(session), t },
          });
        }
      }
    }
  }

  return observations;
};

export const observeObjectDrops: FieldObserver<{ objectID: number }> = (session) => {
  const observations: Observation<{ objectID: number }>[] = [];
  const linkStream = session.functions["GetLootSlotLink"];
  const sourceStream = session.functions["GetLootSourceInfo"];
  if (!linkStream || !sourceStream) return observations;

  for (const slot of getParamKeys(linkStream)) {
    const linkEntries = getStream(session, "GetLootSlotLink", slot);
    const sourceEntries = getStream(session, "GetLootSourceInfo", slot);
    if (!linkEntries || !sourceEntries) continue;

    for (const linkEntry of linkEntries) {
      if (typeof linkEntry.v !== "string") continue;
      const parsedLink = parseItemLink(linkEntry.v);
      if (!parsedLink) continue;

      const itemID = parsedLink.itemID;
      const t = linkEntry.t;

      const sourceAt = sourceEntries.find((e) => e.t === t);
      if (!sourceAt || typeof sourceAt.v !== "object" || !sourceAt.v) continue;

      const source = sourceAt.v as { n?: number; [key: number]: unknown };
      for (let i = 1; i <= (source.n ?? 0); i += 2) {
        const guid = source[i];
        if (typeof guid !== "string") continue;
        const parsed = parseGuid(guid);
        if (parsed.kind === "object" && parsed.id !== null) {
          observations.push({
            entityId: itemID,
            value: { objectID: parsed.id },
            confidence: "high",
            provenance: { session: sessionLabel(session), t },
          });
        }
      }
    }
  }

  return observations;
};
