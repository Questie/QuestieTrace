// Dense indexes over the characters and quests the signal reasons about, so the pairwise passes
// can count with typed arrays instead of maps.

import { classAllowed, raceAllowed } from "../../core/eligibility";
import type { CatalogQuest, QuestCatalog } from "../../core/types";
import type { CharacterHistory } from "./history";

/** Characters sharing race and class: every quest's race/class gate treats them alike. */
export interface Cell {
  key: string;
  raceId: number | undefined;
  classId: number | undefined;
  /**
   * 1 where the cell's race and class may take the quest at that index: the catalog's masks, widened
   * by what characters of the cell actually completed. Catalog masks are sometimes too narrow
   * (Forever's 91743 excludes humans, yet every human on that path completed it).
   */
  allows: Uint8Array;
}

export interface Character {
  history: CharacterHistory;
  cell: Cell;
  /** Quest indexes this character completed. */
  done: Int32Array;
}

export class Population {
  readonly quests: CatalogQuest[];
  readonly indexOf = new Map<number, number>();
  readonly characters: Character[];
  /** Per quest index, the characters that took it (held, accepted or completed). */
  readonly takers: Character[][];
  /** Per quest index, the characters that completed it. */
  readonly completers: Character[][];

  constructor(histories: readonly CharacterHistory[], catalog: QuestCatalog, ignore: ReadonlySet<number>) {
    this.quests = Object.values(catalog.quests)
      .filter((quest) => !ignore.has(quest.id))
      .sort((a, b) => a.id - b.id);
    this.quests.forEach((quest, index) => this.indexOf.set(quest.id, index));

    const cells = new Map<string, Cell>();
    this.characters = histories.map((history) => {
      const { raceId, classId } = history.player;
      const key = `${raceId ?? "?"}:${classId ?? "?"}`;
      let cell = cells.get(key);
      if (!cell) {
        const allows = new Uint8Array(this.quests.length);
        this.quests.forEach((quest, index) => {
          allows[index] = raceAllowed(quest.requiredRaces, raceId) && classAllowed(quest.requiredClasses, classId) ? 1 : 0;
        });
        cell = { key, raceId, classId, allows };
        cells.set(key, cell);
      }
      const done = [...history.done.keys()].map((id) => this.indexOf.get(id)).filter((index) => index !== undefined);
      return { history, cell, done: Int32Array.from(done) };
    });

    this.takers = this.quests.map(() => []);
    this.completers = this.quests.map(() => []);
    for (const character of this.characters) {
      for (const questId of character.history.taken.keys()) {
        const index = this.indexOf.get(questId);
        if (index !== undefined) this.takers[index].push(character);
      }
      for (const index of character.done) {
        this.completers[index].push(character);
        character.cell.allows[index] = 1;
      }
    }
  }

  quest(questId: number): CatalogQuest | undefined {
    const index = this.indexOf.get(questId);
    return index === undefined ? undefined : this.quests[index];
  }
}

/** Race/class allow the quest and the character was seen at or above its required level. */
export function couldTake(character: Character, questIndex: number, quest: CatalogQuest): boolean {
  if (!character.cell.allows[questIndex]) return false;
  const level = character.history.maxLevel;
  return quest.requiredLevel === undefined || level === undefined || level >= quest.requiredLevel;
}
