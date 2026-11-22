# Catching up with the TypeScript extractor

The team keeps developing `tools/corrections-extractor` (TypeScript) until the Rust extractor
replaces it. The Rust extractor must produce the same output files byte for byte, generation
timestamps aside. This file is the procedure for "bring the Rust extractor up to parity with the
TypeScript one": port every TypeScript change since the last parity point, then prove the outputs
are identical.

## Parity point

Master `4dec2ad` (2026-11-22). The NPC and object changes up to it are still being merged on PR
#137. The next catch-up starts after this commit.

## 1. Scope the catch-up

List the TypeScript commits since the parity point, then classify each by what it touches:

```sh
git fetch origin
git log --reverse --format='%h %cs %s' <parity point>..origin/master -- tools/corrections-extractor
git show --stat --format= <sha> -- tools/corrections-extractor
```

- **Port** everything that can change the output:
  - observers (`src/extract/observers/`) and merges (`src/extract/merge/`);
  - record emitters (`src/extract/emit/`), lookups (`src/extract/lookups.ts`,
    `observe-lookups.ts`) and finalize (`src/extract/index.ts`);
  - session filtering (`src/extract/observe.ts`);
  - the run's own messages (`src/cli.ts`);
  - the tests of all of the above.
- **Skip** TypeScript-only infrastructure; the Rust extractor has none of it:
  - the observation store (`src/store/`): Parquet/DuckDB, the observer-version hash, store
    backups, and the unit-token column of ADR 0005 (until audits are decided, PLAN.md
    decision 2);
  - the bucket source (rclone, batches, prefetch) and offline mode;
  - timing logs;
  - moving files between `observers/` and `merge/`: Rust keeps a field's observer, value type
    and merge in one module.
- **Watch for addon schema changes** (`chore: bump schema version`) whose new streams the
  TypeScript side starts reading. `trace-data` decodes everything generically, but each new read
  needs its Rust counterpart.

The report at the end lists both groups, the ported and the skipped commits.

## 2. Bring master in

- Work on the Rust conversion branch: `feat/relations-rs` (PR #137) until it is merged, then a new
  branch from master.
- The owner works in this checkout. Ask before rebasing or merging, and never push or force-push
  without being asked.
- Rebase in a scratch worktree, check the trees, and only then move the branch.
  `--rebase-merges` keeps the branch's merge commits. Commits master already holds (squash-merged
  PRs) are skipped as "previously applied".

```sh
git worktree add --detach <scratch>/rebase-try HEAD
git -C <scratch>/rebase-try rebase --rebase-merges origin/master
new=$(git -C <scratch>/rebase-try rev-parse HEAD)
# Both diffs must be empty. <own paths> are the paths only the branch changes
# (tools/corrections-extractor-rs, PLAN.md, RELATIONS.md on PR #137).
git diff --stat origin/master "$new" -- . ':!tools/corrections-extractor-rs' ':!PLAN.md' ':!RELATIONS.md'
git diff --stat HEAD "$new" -- tools/corrections-extractor-rs PLAN.md RELATIONS.md
git reset --keep "$new"      # refuses if it would lose local changes
git worktree remove <scratch>/rebase-try
```

## 3. Freeze the reference

The reference is master's TypeScript output on the 5% sample. Build it from `origin/master`, not
from `HEAD`. Start it first: it runs about 15 min and peaks at about 19 GB (10,000 submissions,
Nov 2026), so run it in the background, on its own, and port in the meantime.

```sh
ref=<scratch>/ts-master
mkdir -p "$ref"
git archive origin/master tools/corrections-extractor | tar -x -C "$ref"
ln -s "$PWD/tools/corrections-extractor/node_modules" "$ref/tools/corrections-extractor/node_modules"
git rev-parse origin/master >"$ref/COMMIT"
(cd "$ref/tools/corrections-extractor" &&
  TRACE_DATA_DIR=~/projects/trace-data EXTRACT_SAMPLE=5 TRACE_STORE_DIR=<scratch>/ts-store npm run -s extract)
# The output lands in $ref/tools/corrections-extractor/output/sample-5/
```

- **Always set `TRACE_STORE_DIR` to a scratch folder.** Its default, `../trace-session-store`
  next to the checkout, is the real store.
- If master changed `package-lock.json`, run `npm ci` in `tools/corrections-extractor` first.
- **Trace-data keeps growing.** `~/projects/trace-data` is synced from the bucket, and every sync
  grows the 5% sample. Note `stat -c %y ~/projects/trace-data/submissions/*/` when the reference
  starts, and check it before trusting a comparison.
- Hardlinked copies of trace-data cannot go into the scratchpad: `/tmp` is another device.

## 4. Port

Port in master's order, one Rust commit per TypeScript commit:

- Subject: `<type>(corrections-extractor-rs): <the TypeScript subject>`.
- Body: "Ports `<sha>`.", then what changes in the output.
- Give no reason the TypeScript commit or README does not give.

Read `DESIGN.md` ("Behaviour that must match the TypeScript tool exactly") first. These
TypeScript idioms keep coming back:

| TypeScript | Rust |
|---|---|
| `obs.provenance.sessionKey ?? obs.provenance.session` | `obs.session` (`SessionId`, one per folded session key); merge tests set it themselves |
| `Number(text)`, `String(n)` | `js::number`, `js::format_number` |
| `Number.isInteger(v) && v > 0` | `v.fract() == 0.0 && v > 0.0` |
| `if (!id) continue` | skip id 0 (ids are `i64` by then) |
| `Map`/`Set` iteration that reaches the output | `IndexMap`/`IndexSet` in the same insertion order |
| ids filtered, then `.sort((a, b) => a - b)` | a `BTreeMap`/`BTreeSet` or a sort |
| `Set<number>` of quest ids | `HashSet<u64>` of `f64::to_bits` (ids are never NaN or -0) |
| a threshold like `size >= strongest * 0.005` | the same `f64` expression: IEEE gives the same result |
| `session.buildInfo?.["4"]` | `session.field("buildInfo")?.get("4")` (`get` takes JS keys) |
| a merge returning `undefined`, then `onlyDefinedFacts` | a merge returning `Option`, then `only_defined` |
| an id table (world drops, spawn-anywhere NPCs) | copy it under `#[rustfmt::skip]` (keeps the group comments), then diff its ids against the TypeScript file with `grep -oE` |

- **Where code goes.** A field lives in one module under
  `crates/corrections/src/observers/<entity>/`. A merge helper shared by several fields gets its
  own module: `observers/quest/givers.rs` holds TypeScript's `merge/quest/relations.ts`.
- **Files a new field touches.** A new staged field also touches `observe.rs` (`StagedSession`,
  `StagedLookups`), `accumulator.rs`, `memory.rs` (`--stats`) and `finalize.rs`.
- **Tests.** Port each commit's `.test.ts` changes and keep their expectations, rewritten
  fixtures included. In this run, many finishedBy tests moved from `QUEST_TURNED_IN` to
  `QUEST_COMPLETE`.
- **Mutation check.** Check that each new test fails without the change. Revert only the code
  line: `git stash` removes the test along with it.
- **Privacy** (CLAUDE.md). An observer that reads units, tooltips or free text filters them
  exactly as the TypeScript one does. Player names, guilds, realms and player GUIDs never reach
  an output.

## 5. Gate

- **After each commit:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo
  test -p corrections`. Run the whole workspace before finishing.
- **Output:** `TRACE_DATA_DIR=~/projects/trace-data scripts/parity.sh 5
  <ref>/tools/corrections-extractor/output/sample-5`. It runs the Rust extractor (about 1 min,
  6 GB) and diffs every file. The job is done when it prints "Identical".
- **While commits remain unported:** tie each remaining difference to one of them. To see which
  fields differ, run the diff through
  `grep '^[<>]' | sed -E 's/[0-9.]+/N/g' | sort | uniq -c | sort -rn`.
- **A difference left once everything is ported** is a bug in the port. Otherwise it is a
  TypeScript behaviour Rust cannot reproduce; that gets a line under "Known differences" in
  `DESIGN.md`, with the reason.

## 6. Splitting the work

- **Groups.** Group the commits by area: quest givers, NPCs and objects, lookups/items/the rest.
- **Subagents.** Independent groups can go to subagents in worktrees. Each brief carries:
  - its commit list;
  - the other groups' commits, whose differences are not its to fix;
  - the reference path;
  - this file.
- **Integration.** The shared files (`observe.rs`, `finalize.rs`, `accumulator.rs`, `memory.rs`)
  conflict only lightly. Cherry-pick each group in order and gate again.
- **Memory.** This machine has memory errors and 31 GB (RELATIONS.md). Run one TypeScript run at
  a time, never a full-archive run next to anything else, and at most a couple of Rust 5% runs at
  once.

## 7. Finish

- Update the parity point above, and "Where we start" in `PLAN.md`.
- Have a fresh reviewer subagent compare the ported commits with the TypeScript diffs, for edge
  cases the sample does not exercise.
- Report:
  - the ported commits;
  - the skipped commits, and why;
  - the gate result;
  - Rust and TypeScript times on the sample (this run: 56 s against 14 min 52 s).
- Do not push.
