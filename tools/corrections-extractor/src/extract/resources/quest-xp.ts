import { columnIndex, csvFor, parseCsv } from "./csv";

/** Highest difficulty column of the QuestXP table that holds XP; Difficulty_0, _8 and _9 are always 0. */
const FIRST_DIFFICULTY = 1;
const LAST_DIFFICULTY = 7;

/** level -> difficulty -> raw XP, from the Forever QuestXP DB2 export. */
const questXpByLevel = new Map<number, number[]>();
const rows = parseCsv(csvFor("QuestXP"));
const idIndex = columnIndex(rows[0], "ID");
for (const row of rows.slice(1)) {
  const level = Number(row[idIndex]);
  if (!Number.isInteger(level)) continue;
  const values: number[] = [];
  for (let difficulty = FIRST_DIFFICULTY; difficulty <= LAST_DIFFICULTY; difficulty++) {
    values[difficulty] = Number(row[columnIndex(rows[0], `Difficulty_${difficulty}`)]);
  }
  questXpByLevel.set(level, values);
}

/** The rounding the client applies to quest XP (the same as Questie's getAdjustedXP). */
export function roundQuestXp(xp: number): number {
  if (xp <= 100) return 5 * Math.floor((xp + 2) / 5);
  if (xp <= 500) return 10 * Math.floor((xp + 5) / 10);
  if (xp <= 1000) return 25 * Math.floor((xp + 12) / 25);
  return 50 * Math.floor((xp + 25) / 50);
}

/** The raw XP of a quest level and difficulty, or undefined when the table has none. */
export function questXpFor(level: number, difficulty: number): number | undefined {
  const xp = questXpByLevel.get(level)?.[difficulty];
  return xp !== undefined && xp > 0 ? xp : undefined;
}

/** The difficulty whose rounded XP is `shownXp` at `level`, or undefined when none or several match. */
export function questXpDifficulty(level: number, shownXp: number): number | undefined {
  const values = questXpByLevel.get(level);
  if (!values) return undefined;

  let found: number | undefined;
  for (let difficulty = FIRST_DIFFICULTY; difficulty <= LAST_DIFFICULTY; difficulty++) {
    if (values[difficulty] <= 0 || roundQuestXp(values[difficulty]) !== shownXp) continue;
    if (found !== undefined) return undefined;
    found = difficulty;
  }
  return found;
}
