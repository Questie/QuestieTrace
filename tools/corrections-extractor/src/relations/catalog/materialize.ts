// Runs forever-quests.lua inside a QuestieDB checkout and parses its JSON. QuestieDB's own
// loader and correction registry do the layering; this file only finds an interpreter.

import { execFileSync } from "child_process";
import { existsSync } from "fs";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";

const SCRIPT = resolve(dirname(fileURLToPath(import.meta.url)), "forever-quests.lua");

/** One quest as the Lua exporter prints it: requested fields by Questie name, raw values (null for nil holes). */
export type RawQuestRow = Record<string, unknown>;

/** Output of forever-quests.lua; see the header comment there. */
export interface MaterializedQuests {
  /** Apply order: base data, Static Corrections, Derived Passes, Dynamic Corrections. */
  sources: string[];
  /** Quest id -> row after Static Corrections and Derived Passes. */
  quests: Record<string, RawQuestRow>;
  /** Quest id -> distinct rows the faction x class views read, for quests a Dynamic Correction touches. */
  dynamicViews: Record<string, RawQuestRow[]>;
  /** Quest id -> fields an authored forever*Fixes.lua provider replaces, adds to or removes from. */
  authoredFields: Record<string, string[]>;
  /** Quests from the inherited Era baseline (data/Forever rows, Forever/legacy/ providers). The rest are Forever-new. */
  inheritedIds: number[];
}

/** Lua 5.1 interpreters QuestieDB bundles under tools/lua-binary/ (x64 only). */
const BUNDLED_LUA: Partial<Record<NodeJS.Platform, string>> = { linux: "linux-x64/lua", win32: "lua.exe" };

/** QuestieDB's bundled interpreter when it fits this machine, otherwise `lua5.1` from PATH. */
export function luaInterpreter(questieDbDir: string): string {
  const bundled = process.arch === "x64" ? BUNDLED_LUA[process.platform] : undefined;
  const path = bundled && resolve(questieDbDir, "tools/lua-binary", bundled);
  return path && existsSync(path) ? path : "lua5.1";
}

/**
 * @param fields Questie quest field names to export (src/meta/questMeta.lua keys).
 * @param fixturePath Tests only: inline base rows and correction layers instead of QuestieDB's files.
 */
export function materializeForeverQuests(questieDbDir: string, fields: readonly string[], fixturePath?: string): MaterializedQuests {
  if (!existsSync(resolve(questieDbDir, "generator/flavor.lua"))) {
    throw new Error(`${questieDbDir} is not a QuestieDB checkout (no generator/flavor.lua). Set QUESTIEDB_DIR.`);
  }
  const args = [SCRIPT, fields.join(","), ...(fixturePath ? [fixturePath] : [])];
  const stdout = execFileSync(luaInterpreter(questieDbDir), args, {
    cwd: questieDbDir,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  return JSON.parse(stdout) as MaterializedQuests;
}
