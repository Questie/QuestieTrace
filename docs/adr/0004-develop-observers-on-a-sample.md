# 0004: Develop observers on a sample

Status: Accepted · 2026-11-09

## Context
Every observer change re-observes all submissions ([0003](0003-observer-version-invalidates-store.md)), which takes long, all the more when downloading from the bucket ([0002](0002-raw-data-stays-in-bucket.md)). Upcoming observers (quest prerequisites) will change often while being built.

## Decision
`EXTRACT_SAMPLE=<percent>` uses a fixed sample of submissions, chosen by a hash of the submission key. It has its own store (`../trace-session-store/sample-<percent>`) and its own output directory (`output/sample-<percent>`). Develop and check observers on the sample, and run on all data once they are done.

## Consequences
- Each try takes minutes and doesn't touch the full store or its output.
- The same submissions every time, so results can be compared across tries.
- Rare entities may be missing from a small sample.
