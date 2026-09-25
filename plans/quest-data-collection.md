# Plan: Extend QuestieTrace to gather Quest data for the Questie quest database

## Goal

Collect **raw** data about quests so a later offline processing step can build/verify the
Questie quest database (`Questie/Database/<flavor>/<flavor>QuestDB.lua`).

Same rules as the NPC plan (`plans/npc-data-collection.md`): QuestieTrace records **raw WoW
API return values** into streams keyed by `functions[<exact API name>][<literal argument>]`,
appending only on change. All interpretation (joining a quest to its giver, building the
prerequisite/chain graph, deriving flags from frequency, clustering coords) happens
**offline**. Prefer IDs; fall back to a name/text only when no ID is exposed.

## How this fits the existing abstraction (read first)

The quest area is already the most heavily instrumented part of QuestieTrace. Two trackers
cover it:

- **`QuestLog`** (`Modules/Trackers/QuestLog.lua`) — for every quest ID in the log, samples
  raw questID-keyed APIs: `GetQuestLogTitle`, `GetQuestLogQuestText`,
  `C_QuestLog.GetQuestObjectives`, `IsQuestComplete`, `HaveQuestData`,
  `C_QuestLog.IsOnQuest`, `C_QuestLog.IsQuestFlaggedCompleted`, `GetQuestTagInfo`, reward
  money/count/`GetQuestLogRewardInfo`, timers, plus the synthetic `QuestLog` membership
  array. Uses index iteration + delayed re-samples + post-removal probes.
- **`QuestDialog`** (`Modules/Trackers/QuestDialog.lua`) — transient dialog/gossip text:
  `GetTitleText`, `GetQuestText`, `GetObjectiveText`, `GetProgressText`, `GetRewardText`,
  `GetRewardXP`, `GetNumQuestChoices`, `IsQuestCompletable`, `GetGossipAvailableQuests`,
  `GetGossipActiveQuests`, `GetAvailableTitle[i]`, `GetActiveTitle[i]`, etc.
- **`UnitInteraction`** already samples `UnitGUID["npc"]`/`["questnpc"]` at
  `QUEST_DETAIL`/`QUEST_PROGRESS`/`QUEST_COMPLETE`/`GOSSIP_SHOW`, which is the raw signal
  for **who** starts/ends a quest.
- **`CompletedQuests`** tracks the `GetQuestsCompleted` delta set.
- **`Reputation`** tracks `GetFactionInfoByID` (reputation reward/requirement context).

Consequences for this plan (mirroring the NPC plan's discipline):

- **Most quest fields are already captured** and only need offline joining. New work is a
  small set of **raw API streams that are genuinely missing** (reward factions, required
  money, quest-giver-scoped available/active quest lists keyed to the NPC).
- **The prerequisite/chain/exclusivity graph is not a stream** — it is derived offline from
  the sequence of "which quests were available/accepted/completed and when" observations
  that already exist. Do **not** add a "prereq tracker".
- **Do not parse the quest giver GUID into an npcId in-addon.** `UnitInteraction` stores raw
  GUID; offline derives startedBy/finishedBy creature/object IDs.
- Several Questie fields have **no client-side source at all** (see "Known gaps"). Do not
  invent them.

## Target: Questie quest database fields

From `Questie/Database/Classic/classicQuestDB.lua` (`QuestieDB.questKeys`):

| # | Key | Needed? | Raw source | New collection? |
|---|-----|---------|------------|-----------------|
| 1 | `name` | yes | `GetQuestLogTitle[qid]` / `GetTitleText` | **none** (exists) |
| 2 | `startedBy` (creature/object/item) | yes | join qid × `UnitGUID["npc"/"questnpc"]` at `QUEST_DETAIL`/`QUEST_ACCEPTED`; item start = `sourceItemId` | **none** (join); item via #11 |
| 3 | `finishedBy` (creature/object) | yes | join qid × `UnitGUID["npc"/"questnpc"]` at `QUEST_PROGRESS`/`QUEST_COMPLETE`/`QUEST_TURNED_IN` | **none** (join) |
| 4 | `requiredLevel` | best-effort | not in log API; only visible pre-accept in quest detail (no numeric API) | **gap** |
| 5 | `questLevel` | yes | `GetQuestLogTitle[qid]` field 2 | **none** (exists) |
| 6 | `requiredRaces` | offline | player race context (`UnitRace["player"]`) across many traces | **none** (aggregate) |
| 7 | `requiredClasses` | offline | player class context (`UnitClass["player"]`) across many traces | **none** (aggregate) |
| 8 | `objectivesText` | yes | `GetQuestLogQuestText[qid]` (objectives) / `GetObjectiveText` | **none** (exists) |
| 9 | `triggerEnd` | partial | exploration objective text (`GetQuestObjectives` type "event") + player position | **none** (join) |
| 10 | `objectives` (creature/item/object/rep/spell) | yes | `C_QuestLog.GetQuestObjectives[qid]` (+ `GetObjectiveText`) | **none** (exists, live progress) |
| 11 | `sourceItemId` | yes | quest starter item; see Step 3 (`GetQuestLogSpecialItemInfo` / item that started quest) | **new (small)** |
| 12 | `preQuestGroup` | offline | availability/accept/complete sequence | **none** (graph derived) |
| 13 | `preQuestSingle` | offline | same | **none** |
| 14 | `childQuests` | offline | same | **none** |
| 15 | `inGroupWith` | offline | same | **none** |
| 16 | `exclusiveTo` | offline | availability set changes after accept | **none** |
| 17 | `zoneOrSort` | best-effort | quest-log header grouping (`GetQuestLogTitle` isHeader rows) + player zone | **new (small)**: capture header rows |
| 18 | `requiredSkill` | **NO source** | not exposed | **gap** |
| 19 | `requiredMinRep` | **NO source** | not exposed | **gap** |
| 20 | `requiredMaxRep` | **NO source** | not exposed | **gap** |
| 21 | `requiredSourceItems` | **NO source** | not exposed | **gap** |
| 22 | `nextQuestInChain` | offline | chain sequence | **none** (derived) |
| 23 | `questFlags` | partial | `GetQuestLogTitle` frequency/isTask/etc | **none** (partial, exists) |
| 24 | `specialFlags` | partial | frequency (daily/weekly) + repeatable observation | **none** (partial) |
| 25 | `parentQuest` | offline | availability graph | **none** |
| 26 | `reputationReward` | yes | reward factions at turn-in (`GetQuestLogRewardFactionInfo` / `C_QuestLog.GetQuestLogMajorFactionReputationRewards`) | **new** |
| 27 | `breadcrumbForQuestId` | offline | availability graph | **none** |
| 28 | `breadcrumbs` | offline | availability graph | **none** |
| 29 | `extraObjectives` | **NO source** | Questie-specific hidden objectives | **gap** |
| 30 | `requiredSpell` | **NO source** | not exposed | **gap** |
| 31 | `requiredSpecialization` | **NO source** | not exposed | **gap** |
| 32 | `requiredMaxLevel` | **NO source** | not exposed | **gap** |
| 33 | `availableUntilCompleted` | offline | availability graph | **none** |
| 34 | `availableStartingWith` | offline | availability graph | **none** |
| 35 | `requiredRanks` | **NO source** | not exposed | **gap** |
| 36 | `disabledByQuest` | offline | availability graph | **none** |

### The "availability graph" (fields 12–16, 22, 25, 27–28, 33–34, 36)

Questie's prerequisite / chain / exclusivity / breadcrumb / parent relationships are **not**
directly readable. They are reconstructed offline by correlating, across many sessions and
characters:

- which quests were **available** from a giver at a moment (`GetGossipAvailableQuests` /
  `GetAvailableTitle[i]` keyed to `UnitGUID["npc"]`),
- which quests were **accepted** (`QUEST_ACCEPTED` events + `QuestLog` membership),
- which quests were **completed** (`CompletedQuests` delta),
- how a giver's available set **changed** right after an accept/turn-in.

All of these signals already exist (with one gap: the available list is not yet keyed to the
giver NPC — Step 2 fixes that). No new "graph tracker" is needed; the graph is an offline
transform.

## Known gaps (no client-side source — do not invent)

These Questie fields cannot be observed on the Classic client via any quest API and are left
as documented gaps: `requiredLevel` (#4, only shown as red text pre-accept, no numeric API),
`requiredSkill` (#18), `requiredMinRep`/`requiredMaxRep` (#19/#20), `requiredSourceItems`
(#21), `extraObjectives` (#29, Questie-internal), `requiredSpell` (#30),
`requiredSpecialization` (#31), `requiredMaxLevel` (#32), `requiredRanks` (#35). These are
supplied by Questie's own corrections/DBC imports, not by trace data.

`requiredRaces`/`requiredClasses` (#6/#7) have no per-quest API either, but are
**statistically inferable** offline from the player race/class attached to each acceptance
across enough traces — so they are "aggregate", not a hard gap.

## Already collected — no new work (offline joins only)

- `name`, `questLevel`, `objectivesText`, `objectives`, most of `questFlags`/`specialFlags`
  (frequency), reward money/items — all in `QuestLog`.
- Quest-dialog text (title/quest/objective/progress/reward text) — in `QuestDialog`.
- `startedBy`/`finishedBy` — offline join of quest id × `UnitGUID["npc"/"questnpc"]` from
  `UnitInteraction` at the dialog events.
- The whole availability/chain graph — offline over existing accept/available/complete
  signals (plus Step 2's giver-keying).

## Design constraints (follow existing trackers)

- Extend existing trackers where the events/iteration already exist (`QuestLog`,
  `QuestDialog`), rather than adding new event fanouts. Add a new tracker only for a signal
  that has no natural host (none identified here).
- Stream keys are **API name → literal argument** (usually the quest ID or an index). Reward
  factions are `functions["GetQuestLogRewardFactions"][questId]` etc.; giver-scoped available
  lists are keyed by the giver GUID (raw), not a parsed npcId. Reward factions use the
  client-specific APIs named in Step 4 (`GetQuestLogRewardFactionInfo` on Classic;
  `C_QuestLog.GetQuestLogMajorFactionReputationRewards` on mainline).
- Raw API values only, append-on-change via `Core.DeepCompare`, `pcall`-guarded, reusing the
  `SafeScalarCall`/`SafePackedCall`/`ProbeQuest*` helpers already in `QuestLog.lua`.
- New/changed streams documented in `specs/SCHEMA_SPEC.md` §9 and `specs/TRACKER_SPEC.md` §7;
  new events in `specs/EVENT_CATALOG.md`; `AGENTS.md` tracker list if a file is added.
- Tests in `Tests/run.lua` (and/or `*.test.lua`); `lua Tests/run.lua`,
  `busted -p ".test.lua" .`, `luacheck -q .` all pass.

---

## Incremental steps

Ordered smallest-value-first; each is independently shippable. Steps 1–2 unlock the most
Questie fields (givers + graph); Steps 3–5 fill concrete reward/starter gaps.

### Step 1 — Verify giver linkage for `startedBy` / `finishedBy` (verification, likely no code)
- Confirm offline can join, at `QUEST_DETAIL`/`QUEST_ACCEPTED` and
  `QUEST_PROGRESS`/`QUEST_COMPLETE`/`QUEST_TURNED_IN`:
  - the quest id (from `QuestDialog`/`QuestLog`/the event args), and
  - `UnitGUID["npc"]`/`["questnpc"]` (from `UnitInteraction`).
- `UnitInteraction` already registers those events and samples those tokens. If a quest id is
  not sampled synchronously with the giver GUID at `QUEST_DETAIL`, add the minimal missing
  sample to the **existing** tracker (prefer `QuestDialog`, which already fires on those
  events). Add/adjust tests.
- Object/gameobject givers: the giver GUID may be an object GUID — record raw; offline splits
  creature vs object (feeds `creatureStart`/`objectStart` and `creatureEnd`/`objectEnd`).

### Step 2 — Key the giver's available/active quest lists to the giver (unlocks the graph)
- Problem: `QuestDialog` records `GetGossipAvailableQuests`/`GetAvailableTitle[i]` /
  `GetActiveTitle[i]` as **giver-agnostic** streams. For the availability graph and
  `startedBy`, offline needs to know **which NPC** offered which quests.
- Fix (smallest form): ensure the giver GUID is observable at the exact same timestamp as the
  available/active list samples. `UnitInteraction` already samples `UnitGUID["npc"]` on
  `GOSSIP_SHOW`/`QUEST_GREETING`; confirm timestamps line up so offline can join by `t`/`tp`.
- If join-by-timestamp is fragile, add a giver-keyed synthetic stream in `QuestDialog`, e.g.
  `functions["GiverOfferedQuests"][giverGuid]` = the available-title list observed while that
  giver's gossip/greeting was open. Mark it *(synthetic)* in `SCHEMA_SPEC` and justify it
  (same bar as `QuestLog`/`SpellBook`). Prefer join-by-timestamp; only add the synthetic
  stream if the join proves unreliable.
- Tests: open gossip on an NPC with N available quests → available titles + `UnitGUID["npc"]`
  observable at the same `t`.

### Step 3 — `sourceItemId` (quest starter item) (#11, and item `startedBy`)
- A quest can be started by using an item (`itemStart`). The raw signals:
  - The item whose use fired `QUEST_DETAIL` (offline: `UseContainerItem`/`UseItemByName` is
    not hooked here — prefer the log-side signal below).
  - `GetQuestLogSpecialItemInfo(questLogIndex)` / `GetQuestLogSpecialItemCooldown` expose the
    quest's provided/usable item for active quests. Add these as questID-keyed raw streams in
    `QuestLog` (they already iterate the log with an index available).
- Add to `QuestLog.SampleQuestLog` per-quest loop:
  - `functions["GetQuestLogSpecialItemInfo"][questId]` (packed) when the API exists.
- Offline maps the provided item → `sourceItemId`/`itemStart`.
- Tests: active quest with a special item → stream recorded; quest without → nothing.

### Step 4 — `reputationReward` (#26)
- At turn-in / while a quest offers reputation rewards, capture reward factions with the
  client-specific APIs (do **not** use `GetQuestLogRewardFactions(index, questId)` — that
  signature does not exist, and legacy results must not be correlated by quest ID):
  - **Classic:** `GetNumQuestLogRewardFactions()` (no arguments) then
    `GetQuestLogRewardFactionInfo(index)` per index (no questId argument; returns
    `factionID, rewardAmount`) — add as raw streams in `QuestLog` reward probing.
  - **Mainline:** `C_QuestLog.GetQuestLogMajorFactionReputationRewards(questId)`
    (returns `QuestRewardReputationInfo[]` with `factionID` + `rewardAmount`), keyed by
    questId like the existing `GetQuestLogRewardInfo[rewardIndex][questId]` nesting.
  - On the dialog/offer side: `GetNumRewardFactions()` / `GetRewardFactions(index)` at
    `QUEST_COMPLETE` when present; on modern clients, additionally
    `C_QuestOffer.GetQuestOfferMajorFactionReputationRewards()` (no arguments) at
    `QUEST_DETAIL` — add to `QuestDialog` flat/indexed streams when present.
- These APIs are version-gated; use `HasMethod`/`type(_G[...]) == "function"` guards like
  existing code so absent APIs create no empty streams.
- Offline joins reward-faction id + value → `reputationReward`. (Reputation *changes* on
  turn-in are already visible via `Reputation` + `CHAT_MSG_COMBAT_FACTION_CHANGE` as a
  cross-check.)
- Tests: quest with a reward faction → stream recorded; API absent → no stream.

### Step 5 — `zoneOrSort` grouping + `triggerEnd` coords (#17, #9) (best-effort)
- `zoneOrSort`: capture quest-log **header rows** so offline can group quests under their
  zone/sort header. In `QuestLog`, the index scan already sees headers
  (`GetQuestLogTitle` `isHeader=true`); currently only non-header quest IDs are kept. Add a
  synthetic `functions["QuestLogHeaders"]` stream = ordered list of
  `{headerTitle, followingQuestIds}` (or simpler: record each quest's immediately-preceding
  header title). Mark *(synthetic)*; justify in spec. Offline maps header title → area/sort.
- `triggerEnd`: exploration objectives surface as `GetQuestObjectives` entries of type
  "event"/"area" with text; join with `C_Map.GetPlayerMapPosition["player"]` (already
  sampled by `Position`) at the moment the objective flips to complete. No new collection
  beyond confirming the objective-progress + position streams overlap in time.
- Tests: quest log with a header → header association recorded; exploration objective flip →
  objective-complete + position observable at the same time.

### Step 6 — Processing notes + spec/docs consolidation
- Add `specs/QUEST_DATA_MAPPING.md` (mirroring the NPC mapping doc) listing, per questKey,
  the exact stream(s) and the offline transform, and explicitly marking the "Known gaps".
- Update `specs/SCHEMA_SPEC.md` §9 (new reward-faction / special-item / header streams),
  `specs/TRACKER_SPEC.md` §7 (QuestLog/QuestDialog rows), `specs/EVENT_CATALOG.md` if any new
  event is registered, and `AGENTS.md` if a file is added.

---

## Net new addon surface (summary)

- **No new tracker.** All additions extend `QuestLog` and `QuestDialog`, whose event fanouts
  and per-quest iteration already exist.
- New **raw** streams: `GetQuestLogSpecialItemInfo[qid]` (#11), reward-faction streams (#26).
- At most **two justified synthetic streams**: a giver-keyed offered-quest list (Step 2, only
  if timestamp-join is unreliable) and `QuestLogHeaders` (Step 5).
- Everything else (givers, chain/prereq/exclusivity graph, races/classes, triggerEnd) is an
  **offline join** over streams that already exist.

## Explicitly out of scope

- Any in-addon derivation of the prerequisite/chain/exclusivity graph or flag bitmasks.
- Fields with no client source (#4, #18–#21, #29–#32, #35) — supplied by Questie corrections.
- Item stats / vendor prices / world-quest / emissary data (Wowhead-only mining).

## Definition of done (per step)

- Changed tracker still registered via `Core.RegisterTracker`; any new file added to **all**
  `*.toc` files.
- Streams keyed by API name → literal argument; values are raw API returns (synthetic streams
  documented as such in `SCHEMA_SPEC`).
- Version-gated APIs guarded; append-on-change via `Core.DeepCompare`; `pcall`-guarded.
- Specs + `AGENTS.md` updated; tests added; `lua Tests/run.lua`,
  `busted -p ".test.lua" .`, and `luacheck -q .` all pass.
