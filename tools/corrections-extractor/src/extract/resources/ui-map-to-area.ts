// uiMapID -> areaID mapping for the Questie export.
//
// Questie corrections are keyed by AreaTable areaIDs, while trace APIs such as
// C_Map.GetBestMapForUnit report uiMapIDs (e.g. Elwynn Forest is uiMapID 1429
// but areaID 12). The two ID spaces do not overlap for zones, so the export
// must translate before writing zoneID / spawns / triggerEnd keys.
//
// The mapping is derived from the UiMap and AreaTable DB2 CSVs in this
// directory: a UiMap row and an AreaTable row describe the same map when their
// display names match (UiMap.Name_lang vs AreaTable.AreaName_lang). For
// duplicate display names (e.g. sub-area variants like "Stormwind City"
// Camelot), the lowest areaID wins; sub-areas share their parent's map, so the
// base area is the canonical Questie zone. Traces may also contain uiMapIDs
// with no matching area (continents, Azeroth, dungeons) - those have no Questie
// zone and are dropped by the export.

import { columnIndex, csvFor, parseCsv } from "./csv";

/** Normalized display name -> lowest matching AreaTable areaID. */
function buildAreaIdByName(): Map<string, number> {
  const rows = parseCsv(csvFor("AreaTable"));
  const header = rows[0];
  const idIndex = columnIndex(header, "ID");
  const nameIndex = columnIndex(header, "AreaName_lang");
  const byName = new Map<string, number>();
  if (idIndex < 0 || nameIndex < 0) return byName;

  for (const row of rows.slice(1)) {
    const id = Number(row[idIndex]);
    const name = row[nameIndex];
    if (!Number.isFinite(id) || id === 0 || !name) continue;
    const key = name.trim().toLocaleLowerCase();
    const existing = byName.get(key);
    if (existing === undefined || id < existing) byName.set(key, id);
  }
  return byName;
}

/** Normalized display name -> UiMap uiMapID (first row wins). */
function buildUiMapIdByName(): Map<string, number> {
  const rows = parseCsv(csvFor("UiMap"));
  const header = rows[0];
  const idIndex = columnIndex(header, "ID");
  const nameIndex = columnIndex(header, "Name_lang");
  const byName = new Map<string, number>();
  if (idIndex < 0 || nameIndex < 0) return byName;

  for (const row of rows.slice(1)) {
    const id = Number(row[idIndex]);
    const name = row[nameIndex];
    if (!Number.isFinite(id) || id === 0 || !name) continue;
    const key = name.trim().toLocaleLowerCase();
    if (!byName.has(key)) byName.set(key, id);
  }
  return byName;
}

const areaIdByName = buildAreaIdByName();
const uiMapIdByName = buildUiMapIdByName();
const areaIdByUiMapId = new Map<number, number>();
for (const [name, uiMapId] of uiMapIdByName) {
  const areaId = areaIdByName.get(name);
  if (areaId !== undefined) areaIdByUiMapId.set(uiMapId, areaId);
}

/**
 * Translate a uiMapID (as reported by C_Map.GetBestMapForUnit) into the
 * AreaTable areaID used by Questie corrections. Returns null when the uiMapID
 * has no corresponding area (continents, dungeons, unknown ids).
 */
export function areaIdForUiMapId(uiMapId: number | null | undefined): number | null {
  if (typeof uiMapId !== "number" || !Number.isFinite(uiMapId)) return null;
  return areaIdByUiMapId.get(uiMapId) ?? null;
}
