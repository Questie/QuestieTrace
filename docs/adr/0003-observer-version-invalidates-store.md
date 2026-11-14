# 0003: Observer code changes rebuild the store

Status: Accepted · 2026-11-09

## Context
Stored observations are only valid for the code that made them. A forgotten manual version bump would silently mix old and new observations.

## Decision
`src/store/observer-version.ts` hashes the sources observers run: `core/`, `extract/observers/`, `extract/resources/`, the registries `extract/observe.ts` and `extract/observe-lookups.ts`, and their helpers. A different hash empties the store, and the run observes every submission again. A test fails when the observe path imports a file that isn't hashed. The store's own file format has a separate `FORMAT_VERSION`.

## Consequences
- No stale observations, and nothing to remember.
- Merge and emit code (`extract/index.ts`, `extract/merge/`, `lookups.ts`, `emit/`, `schema/`) can change without a rebuild. Observers must not import from `extract/merge/`.
- How the store writes observations (`observed-store.ts`) and npm dependencies used by observers are not hashed. Bump `FORMAT_VERSION` when changing them.
- Developing observers against the full data is slow, see [0004](0004-develop-observers-on-a-sample.md).
