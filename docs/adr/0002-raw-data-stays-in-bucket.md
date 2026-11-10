# 0002: Raw submissions stay in the bucket

Status: Accepted · 2026-11-09

## Context
The local rclone mirror of the bucket reached 141 GB, and the disk ran low. Exports are cumulative, but deduplicating sessions would still leave about 60–95 GB. OVH charges nothing for downloads or API requests.

## Decision
The OVH bucket `trace-intake-ovh` is the only copy of raw submissions, and the extractor's default source. The extractor lists it with rclone (remote `ovhcloud-de`, the same name for the whole team) and downloads only new submissions, in batches into a temporary directory that is removed after each batch. No raw submissions or sessions are kept locally; only their observations are, in `trace-session-store` ([0001](0001-observations-store-in-parquet.md)).

## Consequences
- Local disk use is the observation store (a few GB) plus outputs.
- A full re-run downloads everything again: free, but slower than reading a local copy.
- Submissions deleted from the bucket (rejections) are dropped from the store. Older submissions holding unobserved copies of their sessions are observed again.
- The trace-analyzer still reads a local checkout. Keep the mirror until it can fetch from the bucket.
