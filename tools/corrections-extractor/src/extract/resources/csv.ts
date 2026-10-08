// Shared helpers for the version-suffixed DB2 CSV resources in this directory
// (a build suffix in the filename is ignored on purpose: swapping in a newer
// build requires no code change). Files are loaded eagerly as raw strings.

const csvResources = import.meta.glob("./*.csv", {
  eager: true,
  query: "?raw",
  import: "default",
});

/** Resolve the raw CSV for a table by filename prefix. Exactly one CSV per table is expected. */
export function csvFor(table: string): string {
  const prefix = `./${table}.`;
  for (const key of Object.keys(csvResources)) {
    if (key.startsWith(prefix) && typeof csvResources[key] === "string") return csvResources[key] as string;
  }
  return "";
}

/**
 * Parse the small subset of CSV needed by the DB2 exports. The exported files
 * contain quoted fields (for example, "The Barrens"), so splitting on commas is
 * not sufficient.
 */
export function parseCsv(csv: string): string[][] {
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

/** Read a named column's index from a CSV header row, or -1 when absent. */
export function columnIndex(header: string[] | undefined, column: string): number {
  if (!header) return -1;
  return header.indexOf(column);
}
