// Turns a per-entity record map into a paste-ready Questie "forever*Traces"
// corrections module - a `QuestieLoader` module exposing a single `:Load()`
// function that returns a sparse map of entityId -> { [entityKeys.field] = value }.
//
// Unlike a full DB dump, this only ever emits fields we actually have an
// aggregated Fact for (see emit/npc.ts); entities with no corrected fields are
// omitted entirely, and fields with no observer/no data are never defaulted.
// This matches how Questie's own `Database/Custom/Fixes/foreverNPCFixes.lua`
// (etc.) is structured: a set of overrides layered on top of the base DB, not
// a replacement for it.
//
// The emitted Lua module name and filename follow the pattern `Forever{Entity}Traces`
// / `forever{Entity}Traces.lua` (e.g. `ForeverItemTraces` / `foreverItemTraces.lua`).

export interface CorrectionsWriterHeader {
  sourceFileNames: string[];
  sessionCount: number;
  generatedAt: Date;
}

/** Renders an object key as a Lua table constructor key, e.g. `["creatures"]` or `[7]`. */
function luaKey(key: string): string {
  return /^\d+$/.test(key) ? `[${key}]` : `[${luaString(key)}]`;
}

/** Checks if value looks like a starter/finisher type table with positional arrays. */
function getPositionalArrays(value: unknown): { arrays: number[][]; type: string } | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const keys = Object.keys(value).sort();

  // Check for startedBy pattern: creatures, items, objects
  if (keys.length >= 1 && keys.length <= 3) {
    const validKeys = new Set(["creatures", "items", "objects"]);
    if (keys.every((k) => validKeys.has(k))) {
      const obj = value as { creatures?: number[]; objects?: number[]; items?: number[] };
      // Could be startedBy (3 positions) or questEnds/finishedBy (2 positions)
      if (keys.includes("items")) {
        // Has items -> startedBy pattern
        return { arrays: [obj.creatures ?? [], obj.objects ?? [], obj.items ?? []], type: "startedBy" };
      } else {
        // No items -> finishedBy/questEnds pattern
        return { arrays: [obj.creatures ?? [], obj.objects ?? []], type: "finishedBy" };
      }
    }
  }

  return null;
}

/** Maps internal field names to QuestieDB keys table field names.
 * Some fields use an "_add" suffix in the output to distinguish correction
 * fields from base DB fields (e.g., startedBy vs startedBy_add). */
const QUESTIE_FIELD_MAP: Record<string, string> = {
  startedBy: "startedBy_add",
  finishedBy: "finishedBy_add",
  questStarts: "questStarts_add",
  questEnds: "questEnds_add",
  npcDrops: "npcDrops_add",
  objectDrops: "objectDrops_add",
};

/** Returns the QuestieDB keys table field name for a given internal field name. */
function getQuestieFieldName(fieldName: string): string {
  return QUESTIE_FIELD_MAP[fieldName] ?? fieldName;
}

/** Set of field names to exclude from export. Internal field names. */
const EXPORT_EXCLUDED_FIELDS: Set<string> = new Set([
  "requiredLevel",
  "zoneOrSort",
]);

/** Returns true if the field should be excluded from export. */
function isFieldExcluded(fieldName: string): boolean {
  return EXPORT_EXCLUDED_FIELDS.has(fieldName);
}

/** Escapes a string as a Lua 5.1 string literal body, including control
 * characters (e.g. the embedded \r\n the WoW client returns for some NPC names
 * and gossip text) that would otherwise break the emitted file. Lua 5.1 has no
 * \xHH escape, so unmapped control chars use zero-padded decimal form. */
function luaString(value: string): string {
  const escaped = value
    .replace(/[\\"]/g, "\\$&")
    .replace(/[\b\f\n\r\t\v]/g, (ch) => {
      switch (ch) {
        case "\b": return "\\b";
        case "\f": return "\\f";
        case "\n": return "\\n";
        case "\r": return "\\r";
        case "\t": return "\\t";
        default: return "\\v";
      }
    })
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u0006\u000e-\u001f\u007f]/g, (ch) => `\\${ch.charCodeAt(0).toString().padStart(3, "0")}`);
  return `"${escaped}"`;
}

function luaValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "nil";
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? String(value) : "nil";
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "string") {
    return luaString(value);
  }
  if (Array.isArray(value)) {
    return value.length === 0 ? "nil" : `{${value.map((v) => luaValue(v)).join(",")}}`;
  }
  if (typeof value === "object") {
    // Special case: positional tables for starter/finisher types
    // { creatures: [...], objects: [...], items: [...] } → {{...},{...},{...}}
    // Omit trailing empty arrays (render as nil) to match Questie convention
    const positional = getPositionalArrays(value);
    if (positional) {
      const pos = [...positional.arrays];
      // Omit trailing empty arrays
      while (pos.length > 0 && pos[pos.length - 1].length === 0) {
        pos.pop();
      }
      if (pos.length === 0) return "nil";
      return `{${pos.map((arr) => luaValue(arr)).join(",")}}`;
    }
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      return "nil";
    }

    return `{${entries.map(([key, entryValue]) => {
      const rendered = luaValue(entryValue);
      // Flat arrays of numbers (e.g. quest objectives IDs) must be wrapped in an
      // extra table layer so Questie reads them as {[idx] = {{id1, id2, ...}}}.
      if (Array.isArray(entryValue) && entryValue.every((v) => typeof v === "number")) {
        return `${luaKey(key)}={${rendered}}`;
      }
      return `${luaKey(key)}=${rendered}`;
    }).join(",")}}`;
  }
  throw new Error(`luaValue: unsupported value type ${typeof value} (${JSON.stringify(value)})`);
}

/**
 * @param moduleName e.g. "ForeverNpcTraces" - becomes the QuestieLoader module name.
 * @param keysLocalName e.g. "npcKeys" - the `QuestieDB.<keysLocalName>` field-key table used
 *   to translate each record's field names into their real Questie key lookups.
 * @param records entityId -> only the fields that were actually extracted (already sparse,
 *   see emit/npc.ts); entities with an empty record are skipped.
 */
export function writeQuestieCorrectionsLua(
  moduleName: string,
  keysLocalName: string,
  records: Map<number, Record<string, unknown>>,
  header: CorrectionsWriterHeader,
): string {
  const lines: string[] = [
    `---@class ${moduleName}`,
    `local ${moduleName} = QuestieLoader:CreateModule("${moduleName}")`,
    "",
    "---@type QuestieDB",
    'local QuestieDB = QuestieLoader:ImportModule("QuestieDB")',
    "",
    "-- Generated by QuestieTrace extract (tools/corrections-extractor)",
    `-- Source trace files: ${header.sourceFileNames.length}`,
    `-- Sessions aggregated: ${header.sessionCount}`,
    `-- Generated at: ${header.generatedAt.toISOString()}`,
    `function ${moduleName}:Load()`,
    `    local ${keysLocalName} = QuestieDB.${keysLocalName}`,
    "",
    "    return {",
  ];

  const sortedIds = [...records.keys()].sort((a, b) => a - b);
  for (const id of sortedIds) {
    const record = records.get(id);
    const fieldNames = record ? Object.keys(record) : [];
    if (fieldNames.length === 0) continue;

    lines.push(`        [${id}] = {`);
    for (const fieldName of fieldNames) {
      if (isFieldExcluded(fieldName)) continue;
      const value = record![fieldName];
      let rendered: string;
      // Quest objectives use a positional 3-element table: {creatureObj, objectObj, itemObj}.
      // Missing positions become nil so indices line up (e.g. only items → {nil,nil,{{id}}}).
      // Each objective ID must be wrapped in its own table: {{id1},{id2},...}
      if (fieldName === "objectives" && typeof value === "object" && value !== null && !Array.isArray(value)) {
        const obj = value as Record<number, number[]>;
        const positions: (number[] | null)[] = [obj[1] ?? null, obj[2] ?? null, obj[3] ?? null];
        // Drop trailing nulls only if at least one position is filled
        while (positions.length > 1 && positions[positions.length - 1] === null) {
          positions.pop();
        }
        const renderedPositions = positions.map((p) => p === null ? "nil" : `{${p.map((id) => `{${id}}`).join(",")}}`,
        );
        rendered = `{${renderedPositions.join(",")}}`;
      } else {rendered = luaValue(value);
      }
      const outputFieldName = getQuestieFieldName(fieldName);
      lines.push(`            [${keysLocalName}.${outputFieldName}] = ${rendered},`);
    }
    lines.push("        },");
  }

  lines.push("    }");
  lines.push("end");

  return lines.join("\n");
}
