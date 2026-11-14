# 0006: Merges live outside the observer sources

Status: Accepted · 2026-11-12

## Context
Merge functions sat next to their observers in `extract/observers/`, which the observer version hashes ([0003](0003-observer-version-invalidates-store.md)). Tuning how observations are combined, e.g. dropping quest givers seen in too few sessions, rebuilt the whole store though no observation changed.

## Decision
Merges live in `extract/merge/<entity>/<field>.ts`, observers in `extract/observers/`. Observers record what one session saw. Merges decide what is believed across sessions, including support thresholds such as `MIN_GIVER_SESSIONS` (5) for `startedBy`/`finishedBy`. Observers must not import from `extract/merge/`.

## Consequences
- Thresholds and other merge rules can be tuned with a re-merge, without observing every submission again.
- A field is split over two files, the observer and its merge.
- Quests with a giver seen in fewer than 5 sessions get no `startedBy`/`finishedBy` until more sessions come in.
