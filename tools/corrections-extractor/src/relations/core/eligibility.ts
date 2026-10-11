// Could this character have taken this quest at all, ignoring relationships?
// Signals use this to gate negative evidence ("X was not offered" only means
// something if the character was otherwise eligible for X).

import type { CatalogQuest, PlayerContext } from "./types";

/** Highest quest id in Classic Era data. Higher ids are Forever-new content. */
export const MAX_CLASSIC_QUEST_ID = 9665;

export function isForeverQuestId(questId: number): boolean {
  return questId > MAX_CLASSIC_QUEST_ID;
}

/** Forever builds report interface versions 16xxx; old traces have no version and are kept. */
export function isForeverInterface(interfaceVersion: number | undefined): boolean {
  return interfaceVersion === undefined || String(interfaceVersion).startsWith("16");
}

/**
 * Questie race mask bit per Blizzard race id. Not simply 2^(id - 1): Forever's Skyborne races
 * (95, 96) use bits 32 and 33. Mirrors QuestieDB src/corrections/enum/expansions.lua raceMaskById.
 */
const RACE_MASK_BY_ID: Readonly<Record<number, number>> = {
  1: 1, // Human
  2: 2, // Orc
  3: 4, // Dwarf
  4: 8, // Night Elf
  5: 16, // Undead
  6: 32, // Tauren
  7: 64, // Gnome
  8: 128, // Troll
  9: 256, // Goblin
  10: 512, // Blood Elf
  11: 1024, // Draenei
  22: 2097152, // Worgen
  24: 8388608, // Pandaren
  25: 16777216, // Pandaren (Alliance)
  26: 33554432, // Pandaren (Horde)
  95: 4294967296, // Skyborne (Alliance)
  96: 8589934592, // Skyborne (Horde)
};

/** Every race id Questie masks know about, e.g. for enumerating the races a mask allows. */
export const KNOWN_RACE_IDS: readonly number[] = Object.keys(RACE_MASK_BY_ID).map(Number);

/**
 * Whether `bit` (a power of two) is set in a Questie bitmask. Masks use bits above 32,
 * which JavaScript's `&` and `>>` truncate, so this uses arithmetic.
 */
function hasBit(mask: number, bit: number): boolean {
  return Math.floor(mask / bit) % 2 === 1;
}

/** Undefined or 0 masks mean "everyone"; unknown race ids are treated as allowed. */
export function raceAllowed(mask: number | undefined, raceId: number | undefined): boolean {
  if (!mask || raceId === undefined) return true;
  const bit = RACE_MASK_BY_ID[raceId];
  return bit === undefined ? true : hasBit(mask, bit);
}

/** Class bits are 2^(classId - 1). Undefined or 0 masks mean "everyone". */
export function classAllowed(mask: number | undefined, classId: number | undefined): boolean {
  if (!mask || classId === undefined || classId < 1) return true;
  return hasBit(mask, 2 ** (classId - 1));
}

export interface EligibilityOptions {
  /**
   * Levels below the catalog's requiredLevel to tolerate. Catalog levels are not always exact,
   * so callers checking negative evidence may want to require a margin instead (pass a negative value).
   */
  levelSlack?: number;
}

/**
 * Static eligibility only: race, class and level. Skills, reputation and spells are not
 * tracked per character, so quests with those requirements should be treated as unknown
 * by signals that rely on negative evidence (see hasUntrackedRequirements).
 */
export function isStaticallyEligible(
  quest: CatalogQuest,
  player: PlayerContext,
  level: number | undefined,
  options: EligibilityOptions = {},
): boolean {
  if (!raceAllowed(quest.requiredRaces, player.raceId)) return false;
  if (!classAllowed(quest.requiredClasses, player.classId)) return false;
  if (quest.requiredLevel !== undefined && level !== undefined) {
    if (level + (options.levelSlack ?? 0) < quest.requiredLevel) return false;
  }
  return true;
}

/** Requirements we cannot check from a trace: skill, reputation, spell knowledge. */
export function hasUntrackedRequirements(quest: CatalogQuest): boolean {
  return (
    quest.requiredSkill !== undefined ||
    quest.requiredMinRep !== undefined ||
    quest.requiredMaxRep !== undefined ||
    quest.requiredSpell !== undefined
  );
}
