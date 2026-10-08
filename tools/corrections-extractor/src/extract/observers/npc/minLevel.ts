// npcKeys.minLevel - Questie field 4.
//
// No signal: `Modules/Trackers/UnitLevel.lua` only ever records UnitLevel for the
// "player" token, never for target/npc/questnpc. There is currently no addon-side
// stream that reports an NPC's level. Always returns no observations; the field
// falls back to its schema default at emit time. Revisit if the addon ever adds
// NPC level capture (e.g. via combat log or a dedicated tracker).

import type { FieldObserver } from "../../observation";

export const observeMinLevel: FieldObserver<number> = () => [];
