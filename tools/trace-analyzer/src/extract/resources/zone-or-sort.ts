const csvResources = import.meta.glob("./*.csv", {
  eager: true,
  query: "?raw",
  import: "default",
});

/**
 * Resolve the raw CSV for a table by filename prefix, ignoring the version
 * suffix in the filename (e.g. "AreaTable.1.60.1.69977.csv"). Exactly one CSV
 * per table is expected, so the first match wins.
 */
function csvFor(table: string): string {
  const prefix = `./${table}.`;
  for (const key of Object.keys(csvResources)) {
    if (key.startsWith(prefix) && typeof csvResources[key] === "string") return csvResources[key] as string;
  }
  return "";
}

const areaTableCsv = csvFor("AreaTable");
const questSortCsv = csvFor("QuestSort");
const zoneOrSortByName = new Map<string, number>();
addNames(zoneOrSortByName, areaTableCsv, ["AreaName_lang", "ZoneName"], 1);
addNames(zoneOrSortByName, questSortCsv, ["SortName_lang"], -1);

/**
 * Parse the small subset of CSV needed by the DB2 exports. The exported files
 * contain quoted fields (for example, "The Barrens"), so splitting on commas is
 * not sufficient.
 */
function parseCsv(csv: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;

  for (let i = 0; i < csv.length; i++) {
    const character = csv[i];

    if (inQuotes) {
      if (character === '"' && csv[i + 1] === '"') {
        field += '"';
        i++;
      } else if (character === '"') {
        inQuotes = false;
      } else {
        field += character;
      }
      continue;
    }

    if (character === '"') {
      inQuotes = true;
    } else if (character === ",") {
      row.push(field);
      field = "";
    } else if (character === "\n") {
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
    } else if (character !== "\r") {
      field += character;
    }
  }

  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }

  return rows;
}

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
  if (!header) return;

  const idIndex = header.indexOf("ID");
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
