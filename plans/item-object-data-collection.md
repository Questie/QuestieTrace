# Plan: Extend QuestieTrace to gather Item and Object data for the Questie databases

## Goal

Collect **raw** data about items and world objects (gameobjects) so a later offline step can
build/verify Questie's item DB (`<flavor>ItemDB.lua`) and object DB (`<flavor>ObjectDB.lua`).

Same rules as the NPC and quest plans (`plans/npc-data-collection.md`,
`plans/quest-data-collection.md`): QuestieTrace records **raw WoW API return values** into
streams keyed by `functions[<exact API name>][<literal argument>]`, appending only on change.
All interpretation (parsing an id from a GUID/itemLink, clustering spawn coords, joining a
drop to its source, deriving flags) happens **offline**. Prefer IDs; fall back to name/text
only when no ID is exposed.

## How this fits the existing abstraction (read first)

- **`Loot`** (`Modules/Trackers/Loot.lua`) already samples, per loot slot:
  `GetLootSlotInfo[slot]`, `GetLootSourceInfo[slot]` (**source GUID** → npc/object/item id
  offline), `GetLootSlotLink[slot]` (**itemLink** → item id offline), `GetLootSlotType[slot]`.
  This is the core drop signal for both item drops and object identity.
- **`UnitInteraction`** samples `UnitGUID["npc"/"questnpc"]` at quest + merchant events. It
  registers `MERCHANT_SHOW` **only for NPC identity** — there is **no merchant inventory
  scan** today.
- **`Position`** samples player coords every 0.2s and on movement.
- **`QuestLog`** samples quest reward items (`GetQuestLogRewardInfo`) and (per the quest plan)
  will add the quest-starter special item.
- There is **no** `GetItemInfo` / tooltip / merchant collection anywhere yet.

Consequences (mirroring the prior plans' discipline):

- **Objects are structurally NPCs.** The object DB keys are a subset of the NPC keys. Almost
  everything is already captured as raw GUIDs (loot source, quest giver) or is an offline
  join with `Position`. Reuse the NPC plan's approach; do **not** parse GUIDs in-addon.
- **Item drops / quest rewards / quest starts are offline joins** over streams that already
  exist (`GetLootSourceInfo` + `GetLootSlotLink`, quest reward streams, quest-starter item).
- The genuinely **new** collection is: item **static metadata** (`GetItemInfo`) and
  **vendor inventory** (`GetMerchantItemInfo`). Both are new raw streams on existing events.
- Several item fields (`flags`, `foodType`, `ammoType`) are **not exposed** by Classic item
  APIs — documented gaps supplied by Questie's DBC imports.

---

## Part A — Object database

From `Questie/Database/Classic/classicObjectDB.lua` (`QuestieDB.objectKeys`):

| # | Key | Needed? | Raw source | New collection? |
|---|-----|---------|------------|-----------------|
| 1 | `name` | yes | object name from loot window / interaction (no direct API — see note) | **small** (see A1) |
| 2 | `questStarts` | yes | offline join qid × quest-giver GUID when the giver is an **object** GUID (`UnitInteraction` / soft-interact) | **none** (join) |
| 3 | `questEnds` | yes | same, at turn-in | **none** (join) |
| 4 | `spawns` | yes | offline join **object GUID** (from `GetLootSourceInfo` / interaction) × `C_Map.GetPlayerMapPosition["player"]` | **none** (Position owns coords) |
| 5 | `zoneID` | derived | offline from `spawns` | **none** |
| 6 | `factionID` | best-effort | not exposed for objects | **gap** |
| 7 | `waypoints` | derived | offline from spawns of moving objects (ships/zeppelins) | **none** |

### Object notes

- **Object identity/name (#1).** Classic exposes no `UnitGUID`-style API for gameobjects.
  The raw signals that carry an object id are:
  - `GetLootSourceInfo(slot)` returns a **GameObject-type GUID** when you loot a chest/vein/
    herb — already captured by `Loot`. Offline parses the object id from that GUID.
  - When a quest is started/ended by an object, the giver GUID observed by `UnitInteraction`
    is an object GUID — already captured.
  - The **name** shown while looting an object is not returned by loot APIs. The only place a
    gameobject name is reliably readable is the mouseover/soft-interact tooltip. This is the
    one small addition (A1); everything else is an offline join.
- `factionID` (#6): no client source for gameobjects — documented gap (same as NPC plan).

### Object steps

**A1 — Object name via soft-interact / mouseover tooltip (small, synthetic)**
- Classic Era exposes `GameObject` interaction through soft-targeting on newer clients
  (`PLAYER_SOFT_INTERACT_CHANGED` + the `"softinteract"` unit token) and via the mouseover
  tooltip on all clients.
- Add a tiny tracker (or fold into the NPC plan's `UnitState` tooltip logic) that, on
  `UPDATE_MOUSEOVER_UNIT` / soft-interact, reads the tooltip **first line** (object name) when
  the tooltip target is a gameobject (no `UnitExists`/no player), and records a synthetic
  `functions["ObjectName"][<rawGuidOrToken>]`. Mark *(synthetic)* in `SCHEMA_SPEC`.
- Offline joins object name ↔ object id (from loot/quest-giver GUIDs) by proximity/timestamp.
- Feature-detect the soft-interact token; on clients without it, mouseover tooltip only.
- Tests: mouseover a mocked gameobject tooltip → `ObjectName` recorded; NPC/player → nothing.

**A2 — Verify object spawn + quest linkage (verification, no code)**
- Confirm offline can join object GUID (from `GetLootSourceInfo` and quest-giver GUID) with
  `C_Map.GetPlayerMapPosition["player"]` for `spawns` (#4), and with quest ids for
  `questStarts`/`questEnds` (#2/#3). Both source streams already exist. Close any timestamp
  gap in the **existing** tracker if found.

---

## Part B — Item database

From `Questie/Database/Classic/classicItemDB.lua` (`QuestieDB.itemKeys`):

| # | Key | Needed? | Raw source | New collection? |
|---|-----|---------|------------|-----------------|
| 1 | `name` | yes | `GetItemInfo(id)` (name) / itemLink | **new** (B1) |
| 2 | `npcDrops` | yes | offline join: `GetLootSlotLink[slot]` (item id) × `GetLootSourceInfo[slot]` (creature GUID) | **none** (join) |
| 3 | `objectDrops` | yes | same join where source GUID is an object | **none** (join) |
| 4 | `itemDrops` | yes | container open: item looted from an item (disenchant/lockbox); `GetLootSourceInfo` item GUID / loot-from-item context | **none** (join, best-effort) |
| 5 | `startQuest` | yes | offline: item whose use/possession starts a quest; `GetQuestLogSpecialItemInfo` (quest plan Step 3) + item that fired `QUEST_DETAIL` | **none** (join with quest plan) |
| 6 | `questRewards` | yes | offline join item id × quest id from `GetQuestLogRewardInfo[idx][qid]` (already captured) | **none** (join) |
| 7 | `flags` | **NO source** | not exposed by item API | **gap** |
| 8 | `foodType` | **NO source** | not exposed | **gap** |
| 9 | `itemLevel` | yes | `GetItemInfo(id)` field 4 | **new** (B1) |
| 10 | `requiredLevel` | yes | `GetItemInfo(id)` field 5 (minLevel) | **new** (B1) |
| 11 | `ammoType` | best-effort | `GetItemInfo` does not return ammoType reliably in Classic | **gap** |
| 12 | `class` | yes | `GetItemInfo(id)` classID (field 12) | **new** (B1) |
| 13 | `subClass` | yes | `GetItemInfo(id)` subclassID (field 13) | **new** (B1) |
| 14 | `vendors` | yes | `GetMerchantItemInfo` / `GetMerchantItemLink` at `MERCHANT_SHOW`, joined with `UnitGUID["npc"]` | **new** (B2) |
| 15 | `relatedQuests` | derived | offline union of questRewards + quest objective items + startQuest | **none** (derived) |

### Item notes / gaps

- `flags` (#7), `foodType` (#8), `ammoType` (#11) have **no reliable Classic API** — documented
  gaps, supplied by Questie DBC imports. Do not invent.
- Item **drops** (#2/#3/#4) are the join of the two loot streams that already exist; no new
  collection. The only reason drops might miss data is if `GetItemInfo` hasn't cached the
  name yet — B1 fixes id→metadata resolution, but the **id** itself comes from the itemLink.

### Item steps

**B1 — `ItemInfo` tracker: `GetItemInfo` for observed item ids (static metadata)**
- New file `Modules/Trackers/ItemInfo.lua`.
- Trigger: whenever an item id is **observed** in another stream (loot link, merchant link,
  quest reward, bag). Simplest robust form: sample on `GET_ITEM_INFO_RECEIVED` and on the
  events where item ids appear (`LOOT_READY`, `MERCHANT_SHOW`, quest reward events,
  `BAG_UPDATE`). Maintain a set of "seen item ids" and, for each, record
  `functions["GetItemInfo"][itemId]` (packed tuple) once it resolves.
  - `GetItemInfo` is async: if it returns nil (not cached), do **not** record; the item id is
    queued and re-sampled on `GET_ITEM_INFO_RECEIVED`. This mirrors the existing
    "append only successful observed returns" contract (SCHEMA_SPEC §recording contract).
- Feeds name (#1), itemLevel (#9), requiredLevel (#10), class (#12), subClass (#13).
- How ids are "seen": read them from the item links other trackers already store, or, more
  decoupled, have `ItemInfo` scan `GetLootSlotLink`/`GetMerchantItemLink`/bag links on its own
  events. Prefer the latter to avoid cross-tracker coupling; parse the id from the link with a
  local helper (link parsing is allowed here because the id is the literal argument to the API
  we then call — analogous to `Loot` storing the raw link).
- Tests: mock `GetItemInfo(id)` cached → tuple recorded under `[id]`; uncached (nil) → nothing,
  then `GET_ITEM_INFO_RECEIVED` → recorded.
- TOC + SCHEMA_SPEC §9 (new `GetItemInfo` keyed-by-itemId section) + TRACKER_SPEC §7 row.

**B2 — `Merchant` tracker: vendor inventory scan (#14 vendors)**
- New file `Modules/Trackers/Merchant.lua`.
- Events: `MERCHANT_SHOW`, `MERCHANT_UPDATE`, `MERCHANT_CLOSED`.
- On show/update, iterate `1..GetMerchantNumItems()` and record raw, slot-keyed streams:
  - `functions["GetMerchantItemInfo"][slot]` (packed: name, texture, price, quantity,
    numAvailable, isPurchasable, isUsable, extendedCost, ...)
  - `functions["GetMerchantItemLink"][slot]` (itemLink → item id offline)
- The **vendor NPC** is `UnitGUID["npc"]`, already sampled by `UnitInteraction` on
  `MERCHANT_SHOW` at the same timestamp → offline joins slot item id × vendor npc id →
  `vendors` (#14) and the item's `vendors` reverse-index.
- Do not parse the merchant item id in-addon; store the raw link (like `Loot`).
- Window-lifecycle pattern (like `Loot`): probe known slots on `MERCHANT_CLOSED`, recording
  only observed returns (no invented nils).
- Tests: `MERCHANT_SHOW` with N items → slot info+link recorded; `UnitGUID["npc"]` observable
  at same time; close probes known slots.
- TOC + SCHEMA_SPEC §9 (merchant slot-keyed section) + TRACKER_SPEC §7 row + EVENT_CATALOG
  (`MERCHANT_UPDATE`, `MERCHANT_CLOSED`).

**B3 — Verify drop/reward/start joins (verification, no code)**
- Confirm offline can produce `npcDrops`/`objectDrops`/`itemDrops` (#2–#4) from
  `GetLootSlotLink[slot]` × `GetLootSourceInfo[slot]`, `questRewards` (#6) from
  `GetQuestLogRewardInfo`, and `startQuest` (#5) from the quest plan's special-item stream.
  All source streams already exist; add nothing unless a gap is found.

---

## Net new addon surface (summary)

- **Object DB:** 1 tiny synthetic `ObjectName` capture (A1); everything else is an offline
  join over existing loot/quest-giver GUID + position streams.
- **Item DB:** 2 new trackers — `ItemInfo` (`GetItemInfo[itemId]`) and `Merchant`
  (`GetMerchantItemInfo[slot]` + `GetMerchantItemLink[slot]`). Drops/rewards/starts are
  offline joins over existing streams.
- **No in-addon interpretation:** no GUID→id or link→id parsing beyond obtaining the literal
  argument for the very next raw API call; all clustering/joining/flag derivation is offline.

## Known gaps (no client source — do not invent)

- Object `factionID` (#A6).
- Item `flags` (#7), `foodType` (#8), `ammoType` (#11). Supplied by Questie DBC imports.

## Explicitly out of scope

- Item stats beyond `GetItemInfo` basics (tooltip stat scanning) — Questie doesn't need it.
- Vendor prices as economy data, auction data, stack sizes for pricing — Wowhead-only mining.
- Any in-addon aggregation/averaging/graph derivation.

## Incremental order

1. **A1** Object name (unlocks object identity join).
2. **B2** Merchant scan (unlocks `vendors`, high value, self-contained).
3. **B1** ItemInfo (unlocks static item metadata; async-aware).
4. **A2 / B3** Verification of joins; close any timestamp gaps in existing trackers.
5. Docs: `specs/ITEM_OBJECT_DATA_MAPPING.md`, update SCHEMA_SPEC/TRACKER_SPEC/EVENT_CATALOG,
   `AGENTS.md` tracker list.

## Definition of done (per step)

- New tracker registered via `Core.RegisterTracker`, added to **all** `*.toc` files.
- Streams keyed by API name → literal argument (slot/itemId); values are raw API returns;
  synthetic `ObjectName` documented as such.
- Async `GetItemInfo` nils not recorded until resolved; append-on-change via `Core.DeepCompare`;
  `pcall`-guarded; version-gated APIs guarded.
- Specs + `AGENTS.md` updated; tests added; `lua Tests/run.lua`,
  `busted -p ".test.lua" .`, and `luacheck -q .` all pass.
