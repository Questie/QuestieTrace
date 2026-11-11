# 0005: Store an observation's unit token in its own column

Status: Accepted · 2026-11-11

## Context
`startedBy` and `finishedBy` read the quest giver from `UnitGUID("questnpc")` and fell back to `UnitGUID("npc")`. The token was not stored, so wrong givers (an NPC next to the real finisher, seen through `npc`) could not be traced back to the token that produced them. Only the spawn observers kept their token, inside the `extra` JSON column.

## Decision
`Observation` gets an optional `token`. The store writes it to a nullable `token` column (Parquet, dictionary-encoded) and replays it onto the observation. `extra` keeps whatever else an observer attaches. The spawn observers' token moves from `extra` to the column without code changes. Format version 5.

`startedBy` and `finishedBy` set the token, and only use `npc` for gossip quest lists, where `questnpc` is never set. At `QUEST_DETAIL`, `QUEST_ACCEPTED`, `QUEST_COMPLETE` and `QUEST_TURNED_IN` they use `questnpc` only.

## Consequences
- Which token produced a value is a plain SQL filter: `WHERE token = 'npc'`.
- Values observed through `npc` can be audited and, if too noisy, the gossip fallback dropped.
- The store is rebuilt once, as with any observer change.
- A key shared by several observers and used in filters belongs in a column. One-off details stay in `extra`.
