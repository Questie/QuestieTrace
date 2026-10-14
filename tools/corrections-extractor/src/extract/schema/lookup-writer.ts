// Renders one locale's translations as a Questie l10n lookup module, matching
// the shape of Questie's `Localization/lookups/<Version>/lookup<Entity>/<locale>.lua`.

import { luaString } from "./corrections-writer";

export type LookupTable = "npcNameLookup" | "questLookup" | "itemLookup" | "objectLookup";

function luaLookupValue(value: unknown): string {
  if (value === null || value === undefined) return "nil";
  if (typeof value === "string") return luaString(value);
  if (Array.isArray(value)) return `{${value.map(luaLookupValue).join(",")}}`;
  throw new Error(`Unsupported lookup value: ${JSON.stringify(value)}`);
}

/** Questie writes the outer quest tuple with a space after the comma, unlike every other table. */
function luaEntry(table: LookupTable, value: unknown): string {
  if (table === "questLookup" && Array.isArray(value)) {
    return `{${value.map(luaLookupValue).join(", ")}}`;
  }
  return luaLookupValue(value);
}

export function writeLookupLua(table: LookupTable, locale: string, entries: Map<number, unknown>): string {
  const rows = [...entries.keys()]
    .sort((a, b) => a - b)
    .map((id) => `[${id}] = ${luaEntry(table, entries.get(id))},`);
  return [
    `if GetLocale() ~= ${luaString(locale)} then`,
    "    return",
    "end",
    "",
    "---@type l10n",
    'local l10n = QuestieLoader:ImportModule("l10n")',
    "",
    `l10n.${table}[${luaString(locale)}] = loadstring([[return {`,
    ...rows,
    "}]])",
    "",
  ].join("\n");
}
