# Forever client findings

What was tried on a live Forever client about tooltips, world objects and lookups, and what it showed. It is for anyone extending the trackers or the extractor, so nobody has to find it out again. Each section says where its findings come from.

- **Client**: Forever 1.60.1 (71091), interface 16001, 2026-11-25, out of combat and outside instances unless it says otherwise.
- **How**: `/run` macros typed in chat, and Lua run through a development bridge. Bridge code may be tainted differently from a normal addon, so a *secret* result seen through it should be confirmed from addon code. A value that printed in chat need not be readable by an addon: chat can show a secret string.
- **Traces**: counts come from samples of recorded sessions, named where they are used.
- **Source**: Blizzard's Forever UI source at commit `c01a87b` (build 71091) and Forever's GlobalStrings.

## Object tooltips

`C_TooltipInfo.GetWorldCursor()` and the shown `GameTooltip`'s data are the same, except `dataInstanceID` (a new number for every read) and `lineIndex` (only on the shown one). An object's tooltip (type 4, Object) on the quest object Kobold Tracks:

| Field | Value |
|---|---|
| `guid`, object ID | absent |
| line 1, type 0 | `Kobold Tracks` |
| line 2, type 17 (QuestTitle) | `id` = 91772 |
| line 3, type 8 (QuestObjective) | `2/6 Followed Kobold Tracks`, `numFulfilled` 2, `numRequired` 6, `completed` false |

- The object's name is a type 0 line, not UnitName. A type 0 line can be any text.
- While an object is hovered `mouseover` does not exist and `UnitIsGameObject("mouseover")` is false.
- Type 4 does not mean a usable object. Location captions are type 4 as well, according to Questie's captures (Bloodhoof Village).
- A quest's progress (`C_TooltipInfo.GetQuestPartyProgress(questID)`, type 24) has the same lines: QuestTitle with the ID, then QuestObjective. It returned data without a group.
- A quest link's tooltip (type 23) has `id`, and type 0 lines for the title, `You are on this quest`, the text and the requirements. `PARTY_QUEST_STATUS_ON` (`Nearby party members that are on this quest:`) is the line member names follow.

**Not seen**: the layout in a group. A QuestTitle followed by a QuestPlayer line (type 18, `guid` and name in `leftText`) per member, each with its objectives, comes from Questie's code and tests. Also not seen: which tooltips list a quest only a member has, and tooltips in combat or instances. Questie's audit found object tooltip content secret in combat.

The tooltip types of the running client match the Forever source (`TooltipInfoSharedDocumentation.lua`); the line types were only read from that source, and those seen here (0, 1, 2, 8, 13, 17, 20, 22, 41 and 44) fit it. The copy in `Documentation/WoW-API/Data/Enum.lua` is older: its line types differ from 21 up (`ItemName` is 22 on Forever, 29 in the copy) and it ends at 42, where Forever has 51. The tooltip types differ only in `Outfit` (27).

### When tooltip code runs

Hovering something in the world fires `WORLD_CURSOR_TOOLTIP_UPDATE`, which calls `GameTooltip:SetWorldCursor`, `C_TooltipInfo.GetWorldCursor`, `ProcessInfo`, and then the `TooltipDataProcessor` post-calls. `TOOLTIP_DATA_UPDATE` rebuilds a tooltip and runs them again, which is how late quest lines arrive. The minimap's tooltip is rebuilt **every frame** (`Minimap_OnUpdate`), so a handler must register for single types, never for all of them.

In 1,042 hours of traces (619 sessions over 10 minutes), a session hovered 39 different NPCs and objects (p95 121, most 407) and made 112 mouseover unit changes per hour.

### Minimap tooltips

One minimap tooltip (type 21) was read. Its one line's `leftText` and `leftColor` were secret, also out of combat and through `GetPrimaryTooltipData`, `GetTooltipData` and the shown font string. Nothing else told what the cursor was on. The declaration of `GetMinimapMouseover` does not mention it. `C_Secrets.HasSecretRestrictions()` was true.

## Which object it was

| Interaction | Name | Object ID |
|---|---|---|
| Loot | `UnitNameFromGUID` of the loot source | the loot source's GUID |
| Dialog or quest object | `UnitName("npc")` | `UnitGUID("npc")` |
| Click without loot | `UNIT_SPELLCAST_SENT` target | none |
| Hover only | the object tooltip | none |

- **`UnitNameFromGUID(GetLootSourceInfo(slot))`** named Tin Vein (1732), Copper Vein (1731), Solid Chest (2850) and Wilted Peacebloom (656160) correctly. A made-up GUID gives `UNKNOWNOBJECT`. In 70 loots of creatures it named each. Not tried: combat, dungeons.
- **`UNIT_SPELLCAST_SENT`** had the object's name as `target` for mining (3564), herb gathering (2366), opening a chest (3365) and clicking Kobold Tracks (6247 `Opening`, 5 yards, instant). Through the bridge it was readable in 148 of 148 casts on mobs in combat. For a heal or buff the target is a player's name.
- At the cast, `UnitGUID` of `npc`, `mouseover` and `softinteract` was nil. The object's GUID first appears at `LOOT_READY`. `SoftTargetInteract` was `0` on this client, so `softinteract` is untested.
- **`CHAT_MSG_OPENING`** did not fire for the player's own mining, chest or click. It fired for other players nearby, with their name in the text (`%s performs %s on %s.`) and an empty `playerName`, so `SanitizeChatMsgArgs` does not redact it. Do not record it.
- The loot window's title is the fixed global `ITEMS`, and no loot function returns the source's name. `C_WorldLootObject.GetWorldLootObjectInfoByGUID` has no name.
- Names are not unique: Classic has 8 objects called Solid Chest and 5 called Tin Vein, so an ID must come from a GUID.
- Object IDs in 420 traced sessions (schema 19 and up) came from `GetLootSourceInfo` (252 IDs), `UnitGUID("npc")` (164) and `UnitGUID("questnpc")` (91). `target` and `mouseover` never had one.

Before `UnitNameFromGUID`, the extractor named a looted object by the last object tooltip before the loot. Of 6,886 loots of objects known to Classic, that name was wrong in 307 (4.5%), and after merging 396 of 408 objects were right. For the 619 objects met through `npc` or `questnpc` the name was never wrong, though 22 had none. For clicks without loot, the last tooltip was another object's in 4 of 24 progress moments.

Quest objects to click with no loot, for testing: Kobold Tracks (563442, *Shhh! We're Hunting Kobolds*, 91772), the Attack Plans of *Thwarting Kolkar Aggression* (786) and the valves of *Samophlange* (900). The Kobold Tracking Kit (247970) shows the tracks on the map; it is not used on them.

## Lookups by ID

`C_TooltipInfo.GetHyperlink` answers for things the character has never met. The answer is often empty the first time and there a few seconds later, so retry, or wait for `TOOLTIP_DATA_UPDATE`. Asked for an NPC that does not exist, it stays empty.

| Link | Gives |
|---|---|
| `unit:Creature-0-0-0-0-<NPC ID>-0000000000` | name, title, creature type, `(Elite)` or `(Boss)`. The level is always `??` |
| `quest:<quest ID>:<level>` | title, text and requirements |
| `unit:GameObject-0-0-0-0-<object ID>-0000000000` | nothing, for existing objects too |

- It shows **no rarity**: a rare mob looks normal and a rare elite looks elite. `UnitClassification` of the real unit does tell.
- `C_TooltipInfo.GetItemByID` first answers `Retrieving item information` (line type 41). `GetSpellByID` gives a spell's name, range and cast time.
- Every lookup of something unknown seems to ask the server, so a dump of many should be throttled, and not run on every player's client.

## Spells cast on objects

`Modules/ObjectSpells.lua` lists the spells whose implicit target is a game object (23) or game object or item (26), from `SpellEffect`: 171 in builds 71037 and 71091, and 170 in 70532. `tools/generate-object-spells.py` makes it.

- Effect 33 (Open Lock) alone is no test: 167 of the 171 have it, and 18952 (*Opening Termite Barrel*) has it without targeting an object.
- 47 spells are called `Opening`. Six of them do not target an object, such as 18952, whose target is any unit (25).
- Target 0 is the usual second target and cannot be used to reject spells.
- Their other targets are the caster (1), positions (18, 47, 87) and a nearby object (40). `SpellTargetRestrictions` has no unit flag on any of them (checked once, in a review).
- The client uses target values that `Target.dbde` (WoWDBDefs, 2026-03) does not have: 13, 19, 141, 142, 144, 145, 147, 148, 149 and 153.
- Names differ between builds (2366 is *Herb Gathering* in one and *Herbalism* in another), so match by ID.
- What `target` is for a spell that can also target an item, such as Pick Lock on a traded lockbox, has not been tried.

## Not tried

Combat for object tooltips, loot names and object casts; dungeons; group tooltips; tooltips that list a quest the player is not on; and the recorded code itself in the client.
