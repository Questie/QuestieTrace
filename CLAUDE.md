# QuestieTrace

@AGENTS.md

`AGENTS.md` is the canonical source for repository orientation.

Keep instructions in `AGENTS.md` rather than duplicating them here, so every agent harness
reads the same source.

# Instructions

## Privacy (CRITICAL)

User privacy is of the utmost importance. QuestieTrace must **never** collect or persist:

- Player character names (including the local player's own name), or realm-qualified names
- Guild names
- Player GUIDs (e.g. `Player-1234-0053656F`) — only NPC/Creature/GameObject/Vehicle/Item GUIDs are allowed
- Any free-form text (chat messages, gossip/quest text) that embeds a player name

Rules for any code that touches trackers, dumps, or raw event recording:

- Any GUID captured from a WoW API (`UnitGUID`, `GetLootSourceInfo`, chat message `guid` args, etc.) **must**
  be classified with `Core.ParseGUIDKind` (see `Modules/globals.lua`) before being stored. Discard the value
  entirely if the kind is `"player"` or unrecognized.
- Any free text captured from a WoW API (`C_GossipInfo.GetText`, `GetQuestText`, chat message `text`, etc.)
  **must** be passed through `Core.SanitizeText` before being stored.
- Chat message events (`CHAT_MSG_*`) **must** go through `Core.SanitizeChatMsgArgs` before being recorded to
  `session.events` or dispatched to trackers.
- Never introduce a tracker that stores `UnitName(...)`/`UnitGUID(...)` for a token that can resolve to another
  player without filtering through the helpers above.
- Do not rely on `issecretvalue()` or similar "sometimes correct" global flags as the sole privacy guard —
  privacy filtering must use explicit, deterministic, manually-reviewed checks (e.g. GUID prefix matching,
  known-name substitution) that we own and can test.
- Any new tracker or dump provider must be reviewed against this policy, and covered by a test that proves
  player names/GUIDs are filtered out (see `Tests/run.lua`).

## Lua Standard

- Always create code with LuaLS typing
- Documentation reference can be found at:
  - `./specs/LuaLS_annotations.md`

## Important - Folders

**Specifications:**
  - `./specs` contains program design specifications
  - `./tasks` contain current implementation design documents
  - `./Documentation/WoW-API` contains the full Blizzard UI code and Function Documentation
  - `./Documentation/WoW-Event` contains the almost all Blizzard events for World of Warcraft.
**Code Directories:**
  - `./Trackers` contains all the implementations of different areas we track in World of Warcraft.
**Code Files:**
  - `./QuestieTrace-Classic.toc` World of Warcraft toc file - how files are loaded
  - `./QuestieTrace.lua` - Entrypoint
  - `./globals.lua` - Global variables and constants
  - `./QuestieTrace_StateTracking.lua` - State tracking and management
  - `./QuestieTrace_UI.lua` - User interface code

**Forbidden folders**

- `Trace` contains old code for a Trace UI that is not in use by the code.
- `.shit` this is my manual trashcan, any code from here should never be used.

## World of Warcraft documentation

  - WebSearch `warcraft.wiki.gg` is the most up to date source.
    - e.g. https://warcraft.wiki.gg/wiki/API_GetTimePreciseSec
  - Local full API documentation and Blizzard UI code can be found in `./Documentation/WoW-API`

## Language Server

  - Check code for issues using cli `lua-language-server --check=.` in the root directory.
  - It takes a pretty long time due to blizzard UI so ALWAYS use a 180s (3m) timeout for lua-language-server commands.
