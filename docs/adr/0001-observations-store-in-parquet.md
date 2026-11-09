# 0001: Keep observations in a Parquet store

Status: Accepted · 2026-11-09

## Context
Every extractor run decoded and observed all submissions again (82k files, about 30 min), though only a few thousand are new per day. Observations were written to temporary NDJSON files and deleted after each run.

## Decision
Keep the observations of every processed submission in `../trace-session-store`: a `manifest.json` plus one sessions file and one observations file per batch, as Parquet (zstd) written through DuckDB. A run observes only submissions not in the manifest, then replays all stored observations into the existing merge code, in the same order a full run used.

## Consequences
- Daily runs only decode and observe new submissions.
- Observations can be queried with SQL (DuckDB) without writing an observer.
- New native dependency: `@duckdb/node-api`.
- Batch files only grow. Rows of removed submissions stay until the store is rebuilt (no compaction yet).
