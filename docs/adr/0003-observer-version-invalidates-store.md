# 0003: Observer code changes rebuild the store

Status: Accepted · 2026-11-09

## Context
Stored observations are only valid for the code that made them. A forgotten manual version bump would silently mix old and new observations.

## Decision
`src/store/observer-version.ts` hashes the sources observers run (`core/`, `extract/observers/`, `extract/resources/`, the observer registries and helpers). A different hash empties the store, and the run observes every submission again. The store's own file format has a separate `FORMAT_VERSION`.

## Consequences
- No stale observations, and nothing to remember.
- Merge functions live next to their observers, so changing a merge also forces a full run. That's safe, but costs time.
- Developing observers against the full data is slow, see [0004](0004-develop-observers-on-a-sample.md).
