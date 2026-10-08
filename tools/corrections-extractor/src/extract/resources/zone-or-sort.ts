import { columnIndex, csvFor, parseCsv } from "./csv";

const areaTableCsv = csvFor("AreaTable");
const questSortCsv = csvFor("QuestSort");
const zoneOrSortByName = new Map<string, number>();
addNames(zoneOrSortByName, areaTableCsv, ["AreaName_lang", "ZoneName"], 1);
addNames(zoneOrSortByName, questSortCsv, ["SortName_lang"], -1);

function normalizeName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

function addNames(
  lookup: Map<string, number>,
  csv: string,
  nameColumns: string[],
  sign: 1 | -1,
): void {
  const rows = parseCsv(csv);
  const header = rows[0];
  const idIndex = columnIndex(header, "ID");
  const nameIndexes = nameColumns
    .map((column) => header.indexOf(column))
    .filter((index) => index >= 0);
  if (idIndex < 0 || nameIndexes.length === 0) return;

  for (const row of rows.slice(1)) {
    const id = Number(row[idIndex]);
    if (!Number.isFinite(id) || id === 0) continue;

    for (const nameIndex of nameIndexes) {
      const name = row[nameIndex];
      if (!name) continue;
      const normalizedName = normalizeName(name);
      // Keep the first match. Duplicate display names are not useful for
      // recovering one unambiguous DB2 ID from a quest-log header.
      if (!lookup.has(normalizedName)) {
        lookup.set(normalizedName, sign * id);
      }
    }
  }
}

/**
 * Convert a quest-log header title into the signed Questie zoneOrSort value.
 * Positive values are AreaTable IDs; negative values are QuestSort IDs.
 */
export function zoneOrSortIdForName(name: unknown): number | null {
  if (typeof name !== "string") return null;
  return zoneOrSortByName.get(normalizeName(name)) ?? null;
}
