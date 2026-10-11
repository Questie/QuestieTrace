// Parses WoW GUIDs as they appear in QuestieTrace sessions, e.g.
// "Creature-0-5208-0-7-823-000031" or "GameObject-0-5208-0-7-2843-0000399".
//
// Format (classic): "<Type>-0-<realmID>-<mapID>-<serverID>-<entryID>-<spawnUID>".
// The entity id (npcID/objectID) is always the second-to-last "-"-separated
// segment, and the spawn UID is the last segment - this holds for every kind we
// care about (Creature/Pet/Vehicle/GameObject), so a single generic parser covers
// all of them.
//
// NOTE: per project decision, this module intentionally does NOT filter out
// player GUIDs for privacy reasons - trace files are expected to already be
// sanitized addon-side (see Modules/Privacy.lua). Classification here is purely
// to route a GUID to the right entity kind for extraction, not a privacy guard.

export type GuidKind = "npc" | "object" | "player" | "item" | "unknown";

const KIND_BY_PREFIX: Record<string, GuidKind> = {
  Creature: "npc",
  Pet: "npc",
  Vehicle: "npc",
  GameObject: "object",
  Player: "player",
  Item: "item",
};

export interface ParsedGuid {
  kind: GuidKind;
  /** Entity id (npcID/objectID). Only populated for "npc"/"object" kinds. */
  id: number | null;
  spawnUID: string | null;
}

export function parseGuid(guid: string | null | undefined): ParsedGuid {
  if (typeof guid !== "string" || guid.length === 0) {
    return { kind: "unknown", id: null, spawnUID: null };
  }

  const parts = guid.split("-");
  const prefix = parts[0];
  const kind = KIND_BY_PREFIX[prefix] ?? "unknown";

  if ((kind === "npc" || kind === "object") && parts.length >= 3) {
    const id = Number(parts[parts.length - 2]);
    const spawnUID = parts[parts.length - 1];
    return { kind, id: Number.isFinite(id) ? id : null, spawnUID };
  }

  return { kind, id: null, spawnUID: null };
}
