// ============================================================
// Load WoW Lua SavedVariables files via lua-state
// ============================================================

import {LuaState} from 'lua-state';
import {normalizeLuaValue} from './normalize.js';
import type {TraceFile} from './types.js';

/**
 * Load a QuestieTrace SavedVariables file and return typed data.
 *
 * The file format is: `QuestieTraceCharacter = { ... }`
 * lua-state executes the Lua, then we extract the global.
 */
export function loadTraceFile(filePath: string): TraceFile {
  const lua = new LuaState();
  try {
    lua.evalFile(filePath);

    const raw = lua.getGlobal("QuestieTraceCharacter");

    if (raw === null || raw === undefined) {
      throw new Error(
        `No QuestieTraceCharacter global found in ${filePath}. ` +
          `Is this a valid QuestieTrace SavedVariables file?`
      );
    }

    return normalizeLuaValue(raw) as TraceFile;
  } finally {
    // Frees the native Lua VM right away instead of whenever the GC finalizes it.
    lua.close();
  }
}
