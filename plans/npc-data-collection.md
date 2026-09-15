# Plan: Extend QuestieTrace to gather NPC (creature) data for the Questie NPC database

## Goal

Collect **raw** data about NPCs (creatures) so a later offline processing step can
build/verify the Questie NPC database (`Questie/Database/<flavor>/<flavor>NpcDB.lua`).

QuestieTrace never transforms data. Per `specs/SCHEMA_SPEC.md` and `specs/TRACKER_SPEC.md`,
a tracker records **raw WoW API return values** into streams keyed by
`functions[<exact API name>][<literal argument passed>]`, appending only on change. All
interpretation (parsing npcId from a GUID, averaging spawn coords, deriving zoneID,
reconstructing waypoints, min/max level, role→flag bitmask) happens **offline**, outside
the addon.

Prefer IDs. When the client does not expose an ID, record the raw value that does exist
(e.g. the GUID string, or a name) so offline processing can resolve the ID afterwards.

## How this fits the existing abstraction (read first)

The architecture has one shape (see `specs/SCHEMA_SPEC.md` §5/§9 and the `UnitInteraction`
and `Position` trackers):

- **Streams are keyed by API name + literal argument.** e.g. `functions["UnitLevel"]["player"]`,
  `functions["UnitGUID"]["target"]`, `functions["GetLootSourceInfo"][slot]`.
- **Values are the raw (packed) API return.** Interpretation is deferred to offline.
- **Synthetic streams are the rare, explicitly-flagged exception** (only `QuestLog`,
  `FactionOrder`, `SpellBook` today). Adding one requires justification in the spec.
- **Duplication is avoided by fanning ONE tracker over multiple unit tokens**, not by
  adding one tracker per data field. `UnitInteraction` is the canonical example: a single
  tracker samples `UnitGUID`+`UnitName` for the tokens `target`/`npc`/`questnpc`.

Consequences for this plan:

- Almost all NPC data Questie needs is just **more Unit\* API streams on the `target` and
  `mouseover` tokens**. This is one new tracker, mirroring `UnitInteraction`.
- **Spawns, npcFlags, and quest-giver linkage require NO new collection.** They are offline
  joins over streams that already exist (see "Already collected — no new work" below).
- **npcId is never parsed in-addon.** We store raw `UnitGUID`; offline derives the id.

## Target: Questie NPC database fields

From `Questie/Database/Classic/classicNpcDB.lua` (`QuestieDB.npcKeys`):

| # | Key | Needed? | Raw stream that feeds it | New collection? |
|---|-----|---------|--------------------------|-----------------|
| 1 | `name` | yes | `UnitName["target"]` / `UnitName["mouseover"]` | add `mouseover` token |
| 2 | `minLevelHealth` | **NO** | — | excluded by request |
| 3 | `maxLevelHealth` | **NO** | — | excluded by request |
| 4 | `minLevel` | yes | `UnitLevel["target"]` / `["mouseover"]` | **new stream (existing pattern)** |
| 5 | `maxLevel` | yes | same `UnitLevel` stream | offline min/max over observations |
| 6 | `rank` | yes | `UnitClassification["target"]` / `["mouseover"]` | **new stream** |
| 7 | `spawns` | yes | offline join `UnitGUID["target"]` × `C_Map.GetPlayerMapPosition["player"]` | **none** (Position owns coords) |
| 8 | `waypoints` | derived | offline from many `spawns` observations | **none** |
| 9 | `zoneID` | derived | offline from `spawns` | **none** |
| 10 | `questStarts` | yes | offline join quest id × `UnitGUID["npc"/"questnpc"]` | **none** (already captured) |
| 11 | `questEnds` | yes | offline join quest id × `UnitGUID["npc"/"questnpc"]` | **none** (already captured) |
| 12 | `factionID` | best-effort | not exposed by client; see note | **none** (known gap) |
| 13 | `friendlyToFaction` | yes | `UnitReaction["player"]["target"]` + player faction | **new stream** |
| 14 | `subName` | yes | tooltip line 2 (synthetic) | **new synthetic stream (needs spec sign-off)** |
| 15 | `npcFlags` | yes | offline join interaction events × `UnitGUID["npc"]` | **none** (already captured) |

### Faction note (`factionID`)

The Classic client does **not** expose a creature's `factionID` (FactionTemplate) via any
Unit API. Do not invent it. The observable signal is `UnitReaction("player", unit)` plus the
player's own faction (already in PlayerIdentity). That is enough for offline processing to
derive `friendlyToFaction` (field #13), but **not** the numeric `factionID` (#12). Leave
`factionID` as a documented gap.

## Already collected — no new work (offline joins only)

These Questie fields need **zero** new addon code; they are produced by joining existing
streams by timestamp offline. Step 7 only *verifies* the joins are possible and closes any
synchronization gap.

- **`spawns`/`waypoints`/`zoneID`**: `UnitGUID["target"]` (→ npcId offline) joined with
  `C_Map.GetPlayerMapPosition["player"]` + `C_Map.GetBestMapForUnit["player"]`, both already
  sampled by `Position` every 0.2s and on movement. Do **not** add a spawn tracker; that
  would duplicate position sampling `Position` owns.
- **`npcFlags`**: `MERCHANT_SHOW`/`TRAINER_SHOW`/`TAXIMAP_OPENED`/`BANKFRAME_OPENED`/
  `GOSSIP_SHOW`/etc. are already registered by `UnitInteraction`, which samples
  `UnitGUID["npc"]` at those moments. Offline maps (event that fired near an `npc` GUID) →
  flag bit. Do **not** add a role tracker.
- **`questStarts`/`questEnds`**: `UnitInteraction` samples `UnitGUID["npc"]`/`["questnpc"]`
  on quest dialog events; `QuestDialog`/`QuestLog` sample the quest id. Offline joins them.

## Design constraints (follow existing trackers)

- New tracker uses `Core.RegisterTracker({ events, Init, OnEvent })` exactly like
  `Modules/Trackers/UnitInteraction.lua` (reuse its `SafeScalarCall`/`SafePackedCall` and
  append-on-change-with-`DeepCompare` style).
- Stream keys are **API name → literal token** (e.g. `functions["UnitLevel"]["target"]`),
  never a parsed npcId. `UnitReaction` takes two units, so it is nested in native argument
  order: `functions["UnitReaction"]["player"]["target"]` (schema already supports nested
  parameter maps, SCHEMA_SPEC §5).
- Only sample when the token unit is a **non-player creature**: guard with
  `UnitExists(token) and not UnitIsPlayer(token)`. Do **not** parse the GUID to decide;
  store the raw `UnitGUID` and let offline reject non-creatures. (The player/pet guard is a
  sampling optimization, not interpretation.)
- 2-space indent, double quotes, LuaCATS annotations, `pcall`-guarded API calls.
- New tracker added to **all** `*.toc` files and covered by tests in `Tests/run.lua`
  (and/or a `*.test.lua`). Update `specs/SCHEMA_SPEC.md`, `specs/TRACKER_SPEC.md`,
  `specs/EVENT_CATALOG.md`, and the `AGENTS.md` tracker list.

---

## Incremental steps

Each step is independently shippable. The core is Steps 2–3 (one tracker, grown twice).
Steps 1, 4–6 are small and optional-order after Step 3.

### Step 1 — Add `mouseover` token to `UnitInteraction` (tiny, no new tracker)
- `UnitInteraction` already fans `UnitGUID`/`UnitName` over `target`/`npc`/`questnpc`.
- Add `"mouseover"` to its `TOKENS` list and register `UPDATE_MOUSEOVER_UNIT`.
- This gives offline a second, high-frequency identity source (name + GUID) for NPCs the
  player merely hovers, feeding `name` (#1) and the spawn join (#7) for non-targeted NPCs.
- Tests: mouseover an NPC → `UnitGUID["mouseover"]`/`UnitName["mouseover"]` recorded.
- Update SCHEMA_SPEC unit-token table + TRACKER_SPEC UnitInteraction row.

### Step 2 — New `UnitState` tracker: `UnitLevel` + `UnitClassification` over NPC tokens
- New file `Modules/Trackers/UnitState.lua`, modeled on `UnitInteraction.lua`.
- Tokens: `{ "target", "mouseover" }`.
- Events: `PLAYER_TARGET_CHANGED`, `UPDATE_MOUSEOVER_UNIT`.
- Guard each sample with `UnitExists(token) and not UnitIsPlayer(token)`.
- Streams (raw, append-on-change), keyed by token:
  - `functions["UnitLevel"][token]` → number (feeds #4/#5; record `-1` for "??" raw).
  - `functions["UnitClassification"][token]` → string ("normal"/"elite"/"rare"/
    "rareelite"/"worldboss") (feeds #6).
- Tests: target an NPC (level + classification recorded); mouseover an NPC; target the
  player (nothing recorded); classification/level change appends once.
- TOC: add `Modules/Trackers/UnitState.lua`. Update SCHEMA_SPEC §9 + TRACKER_SPEC §7 table.

### Step 3 — Extend `UnitState` with `UnitReaction` (feeds `friendlyToFaction`)
- Same tracker, same events/guard.
- Add nested stream `functions["UnitReaction"]["player"][token]` → number 1..8.
  (Native argument order: `UnitReaction("player", token)`.)
- Player faction already comes from PlayerIdentity (`UnitFactionGroup["player"]`); do not
  duplicate. Offline joins reaction + player faction → `friendlyToFaction` (#13).
- Tests: NPC with known reaction recorded under `["player"][token]`; no player sampling.
- Update SCHEMA_SPEC nested-parameterized section + TRACKER_SPEC row.

### Step 4 — `subName` via tooltip scan (the only synthetic stream; get sign-off)
- **Design decision required before coding:** `subName` is not a raw API return; it is the
  second `GameTooltip` left line while the unit tooltip is shown. This must be a documented
  *synthetic* stream, like `QuestLog`/`SpellBook`. Confirm this is acceptable and name it
  clearly, e.g. `functions["UnitSubName"][token]` marked *(synthetic)* in SCHEMA_SPEC §9.
- Implementation: an owned hidden `GameTooltip` (`SetOwner(UIParent, "ANCHOR_NONE")`),
  `SetUnit(token)`, read `<tooltipName>TextLeft2`. Record the text only when it looks like a
  title (wrapped in `<...>` or clearly not the unit name / not a level line). Raw text only;
  no parsing of the title into flags.
- Guard: NPC-only (`not UnitIsPlayer`), pcall-wrapped, keep tooltip logic isolated.
- Tests: mock tooltip with `<Weapon Vendor>` line → recorded; NPC with no title → nothing;
  player unit → nothing.
- Fold into `UnitState` (same events/tokens) to avoid a second event fanout.

### Step 5 — Verify quest-giver + npcFlags joins (verification, likely no code)
- Confirm offline can attribute `questStarts`/`questEnds` (#10/#11) by joining the quest id
  (from `QuestDialog`/`QuestLog`) with `UnitGUID["npc"]`/`["questnpc"]` (from
  `UnitInteraction`) at `QUEST_DETAIL`/`QUEST_PROGRESS`/`QUEST_COMPLETE`.
- Confirm `npcFlags` (#15) can be attributed by joining interaction open-events with
  `UnitGUID["npc"]`.
- If (and only if) a synchronization gap exists — e.g. the quest id is not sampled in the
  same event as the npc GUID — add the minimal missing sample to the **existing** tracker
  rather than creating a new one. Add/adjust tests.
- Gameobject quest-givers are out of scope for the NPC DB.

### Step 6 — Processing notes + spec/docs consolidation
- Add a short note (in `Documentation/` or a new `specs/NPC_DATA_MAPPING.md`) listing, per
  Questie npcKey, the exact stream(s) that feed it and the offline transform:
  - level min/max ← min/max of `UnitLevel[token]` observations
  - rank ← `UnitClassification[token]`
  - name ← `UnitName[token]`
  - spawns/waypoints/zoneID ← `UnitGUID[token]` × player position streams, clustered
  - friendlyToFaction ← `UnitReaction["player"][token]` + `UnitFactionGroup["player"]`
  - subName ← `UnitSubName[token]` (synthetic)
  - questStarts/questEnds ← quest id × `UnitGUID["npc"/"questnpc"]`
  - npcFlags ← interaction events × `UnitGUID["npc"]` → bitmask
  - factionID ← **known gap** (not exposed by client)
- Update `specs/EVENT_CATALOG.md` with `UPDATE_MOUSEOVER_UNIT` (if newly added) and confirm
  `PLAYER_TARGET_CHANGED` coverage. Update `AGENTS.md` tracker list with `UnitState`.

---

## Net new addon surface (summary)

- **1 new tracker** (`UnitState`): `UnitLevel`, `UnitClassification`, `UnitReaction`, and the
  synthetic `UnitSubName`, all over `target`/`mouseover`.
- **1 small extension** to `UnitInteraction` (add `mouseover` token).
- **0 new spawn/role trackers** — those are offline joins over existing streams.

This keeps the "one tracker fans over unit tokens" shape, avoids duplicating position/GUID
sampling, and introduces exactly one justified synthetic stream (`UnitSubName`).

## Explicitly out of scope (not needed by Questie NPC DB)

- `minLevelHealth` / `maxLevelHealth` (excluded by request).
- Creature health/power/sex/PvP flag, spell/ability lists (Wowhead-only data mining).
- Any in-addon aggregation/averaging/derivation, or GUID→id parsing — all offline.

## Definition of done (per step)

- New/changed tracker registered via `Core.RegisterTracker`, added to **all** `*.toc` files.
- Streams keyed by API name → literal token; values are raw API returns (only `UnitSubName`
  is synthetic and documented as such).
- Only non-player creature units sampled; API calls pcall-guarded; append-on-change via
  `Core.DeepCompare`.
- Specs updated (`SCHEMA_SPEC`, `TRACKER_SPEC`, `EVENT_CATALOG`) and `AGENTS.md` tracker list.
- Tests added; `lua Tests/run.lua`, `busted -p ".test.lua" .`, and `luacheck -q .` all pass.
