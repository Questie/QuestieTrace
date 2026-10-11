// `npm run relations -- ingest [--force] [--jobs n]`: trace-data submissions -> episodes.jsonl.
//
// Phase 1 decodes and distills each submission in worker threads and caches the result per
// submission (submission.ts), so a re-run only decodes new submissions; --force rebuilds every
// cache entry. Phase 2 streams the cache: it merges each session's copies (merge.ts), resolves
// legacy greeting titles (titles.ts), drops sessions without quest data, groups sessions into
// characters (characters.ts) and writes episodes plus reports/ingest.json.

import { closeSync, existsSync, openSync, readSync } from "fs";
import { availableParallelism } from "os";
import { resolve } from "path";
import { Worker } from "worker_threads";
import { listSubmissionFiles, type SubmissionFile } from "../../core/trace-data/submissions";
import { writeEpisodes, writeJsonAtomic } from "../core/io";
import { paths } from "../core/paths";
import type { QuestEpisode, QuestEventKind } from "../core/types";
import { assignCharacters, CHARACTER_PARAMS, orderSessions, type CharacterInput } from "./characters";
import { DISTILL_PARAMS } from "./distill";
import { SessionMerger, type MergedSession } from "./merge";
import { CACHE_VERSION, cachePath, readCachedResult } from "./submission";
import { emptyGreetingStats, resolveGreeting, TitleIndex } from "./titles";
import { emptyDistillStats, type DistillStats, type SubmissionResult } from "./types";
import type { WorkerReply, WorkerRequest } from "./worker";

export async function run(args: string[]): Promise<void> {
  const force = args.includes("--force");
  const jobsIndex = args.indexOf("--jobs");
  const jobs = jobsIndex >= 0 ? Math.max(1, Number(args[jobsIndex + 1]) || 1) : Math.max(1, Math.min(6, availableParallelism() - 2));
  const started = Date.now();

  const submissions = listSubmissionFiles(paths.traceDataDir);
  const todo = force ? submissions : submissions.filter((submission) => !hasCurrentCache(submission.id));
  console.log(`ingest: ${submissions.length} submissions, ${todo.length} to decode with ${jobs} worker(s)`);
  const crashes = await decodeAll(todo, jobs);
  console.log(`ingest: decoded in ${((Date.now() - started) / 1000).toFixed(0)}s, merging`);

  const report = buildEpisodes(submissions, todo.length, crashes);
  await writeEpisodes(report.episodes);
  const path = resolve(paths.reportsDir, "ingest.json");
  const summary = { generatedAt: new Date().toISOString(), seconds: Math.round((Date.now() - started) / 1000), ...report.summary };
  writeJsonAtomic(path, summary);
  console.log(JSON.stringify(summary, null, 2));
  console.log(`ingest: wrote ${report.episodes.length} episodes to ${paths.episodes} and ${path}`);
}

// ---------------------------------------------------------------------------
// Phase 1: decode + distill in worker threads
// ---------------------------------------------------------------------------

/** Reads only the head of a cache file: `cacheVersion` is its first key. */
function hasCurrentCache(id: string): boolean {
  const path = cachePath(id);
  if (!existsSync(path)) return false;
  const fd = openSync(path, "r");
  try {
    const head = Buffer.alloc(64);
    const length = readSync(fd, head, 0, head.length, 0);
    return head.toString("utf8", 0, length).startsWith(`{"cacheVersion":${JSON.stringify(CACHE_VERSION)},`);
  } finally {
    closeSync(fd);
  }
}

/** Caps each worker's heap so one runaway submission fails alone instead of taking the machine down. */
const WORKER_LIMITS = { maxOldGenerationSizeMb: 2048 };

function startWorker(): Worker {
  return new Worker(new URL("./worker.ts", import.meta.url), { resourceLimits: WORKER_LIMITS });
}

/** Runs one submission; `dead` means the worker itself failed and must be replaced. */
function runInWorker(worker: Worker, submission: SubmissionFile): Promise<{ reply: WorkerReply; dead: boolean }> {
  return new Promise((resolveRun) => {
    const finish = (reply: WorkerReply, dead: boolean) => {
      worker.off("message", onMessage).off("error", onError).off("exit", onExit);
      resolveRun({ reply, dead });
    };
    const onMessage = (reply: WorkerReply) => finish(reply, false);
    const onError = (error: Error) => finish({ path: submission.path, crash: error.message }, true);
    const onExit = (code: number) => finish({ path: submission.path, crash: `worker exited with code ${code}` }, true);
    worker.on("message", onMessage).on("error", onError).on("exit", onExit);
    worker.postMessage({ path: submission.path, cacheId: submission.id } satisfies WorkerRequest);
  });
}

/**
 * Decodes submissions on `jobs` workers. A submission that crashes its worker is reported and
 * left uncached (the next run retries it); the worker is replaced and the run goes on.
 */
async function decodeAll(todo: SubmissionFile[], jobs: number): Promise<string[]> {
  const crashes: string[] = [];
  let next = 0;
  let done = 0;
  const started = Date.now();
  const lane = async () => {
    let worker = startWorker();
    try {
      while (next < todo.length) {
        const submission = todo[next++];
        const { reply, dead } = await runInWorker(worker, submission);
        if (reply.crash) crashes.push(`${submission.id}: ${reply.crash}`);
        if (dead) {
          await worker.terminate();
          worker = startWorker();
        }
        if (++done % 250 === 0 || done === todo.length) {
          console.log(`ingest: ${done}/${todo.length} decoded (${((Date.now() - started) / 1000).toFixed(0)}s)`);
        }
      }
    } finally {
      await worker.terminate();
    }
  };
  await Promise.all(Array.from({ length: Math.min(jobs, todo.length) }, lane));
  return crashes;
}

// ---------------------------------------------------------------------------
// Phase 2: merge, resolve, group, report
// ---------------------------------------------------------------------------

function count<K extends string>(target: Partial<Record<K, number>>, key: K, by = 1): void {
  target[key] = (target[key] ?? 0) + by;
}

function addStats(target: DistillStats, source: DistillStats): void {
  for (const key of Object.keys(target) as (keyof DistillStats)[]) target[key] += source[key] ?? 0;
}

/** Applies a completed timeline's changes to its initial set; sorted ascending. */
function finalCompleted(episode: MergedSession["session"]["episode"]): number[] | undefined {
  if (!episode.completed) return undefined;
  const set = new Set(episode.completed.initial);
  for (const change of episode.completed.changes) {
    for (const id of change.add ?? []) set.add(id);
    for (const id of change.remove ?? []) set.delete(id);
  }
  return [...set].sort((a, b) => a - b);
}

/** The session name is the local wall-clock time the session was saved, "YYYY-MM-DD_HH-MM-SS". */
function sessionNameTime(name: string | undefined): number | undefined {
  const match = name?.match(/^(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})$/);
  if (!match) return undefined;
  const [, y, mo, d, h, mi, s] = match.map(Number);
  return Date.UTC(y, mo - 1, d, h, mi, s) / 1000;
}

/**
 * Approximate epoch start of each session, for ordering a contributor's sessions. GetServerTime
 * at capture start when recorded (nearly always); otherwise the save stamp shifted by the
 * contributor's usual local-clock offset, or the first submission's receive time.
 */
function sessionOrderTimes(sessions: MergedSession[]): Map<string, number> {
  const offsets = new Map<string, number[]>();
  for (const merged of sessions) {
    const { startServerTime, episode } = merged.session;
    const saved = sessionNameTime(episode.sessionName);
    if (startServerTime === undefined || saved === undefined) continue;
    const list = offsets.get(merged.contributorId) ?? offsets.set(merged.contributorId, []).get(merged.contributorId)!;
    list.push(saved - (startServerTime + episode.duration));
  }
  const median = (values: number[]) => [...values].sort((a, b) => a - b)[values.length >> 1];
  const result = new Map<string, number>();
  for (const merged of sessions) {
    const { startServerTime, episode } = merged.session;
    const saved = sessionNameTime(episode.sessionName);
    const offset = offsets.get(merged.contributorId);
    const order =
      startServerTime ??
      (saved !== undefined ? saved - (offset ? median(offset) : 0) - episode.duration : Date.parse(merged.firstReceivedAt) / 1000 - episode.duration);
    result.set(episode.key, Number.isFinite(order) ? order : 0);
  }
  return result;
}

function hasQuestData(merged: MergedSession): boolean {
  const { episode } = merged.session;
  return !!episode.completed || episode.questEvents.length > 0 || episode.offers.length > 0 || episode.questLog.some((entry) => entry.v.length > 0);
}

function buildEpisodes(submissions: SubmissionFile[], decodedThisRun: number, crashes: string[]) {
  const merger = new SessionMerger();
  const titles = new TitleIndex();
  const failures: Array<{ submissionId: string; error: string }> = [];
  let exports = 0;
  let withMultipleExports = 0;

  for (const submission of submissions) {
    const result: SubmissionResult | undefined = readCachedResult(submission.id);
    if (!result) {
      failures.push({ submissionId: submission.id, error: "no cache entry (worker crashed?)" });
      continue;
    }
    if (result.error) failures.push({ submissionId: result.submissionId, error: result.error });
    exports += result.exports;
    if (result.exports > 1) withMultipleExports++;
    for (const session of result.sessions) {
      for (const observation of session.titles) {
        titles.add(observation);
        titles.noteSighting(observation.locale, observation.title, result.contributorId);
      }
      for (const pending of session.pendingGreetings) {
        for (const title of [...pending.available, ...pending.active]) titles.noteSighting(session.episode.locale, title, result.contributorId);
      }
      // Titles are indexed above; holding them for every kept session only costs memory.
      session.titles = [];
      merger.add(result, session);
    }
  }

  const skipped: Partial<Record<"noEvents" | "noQuestData", number>> = {};
  const greetings = emptyGreetingStats();
  const kept: MergedSession[] = [];
  for (const merged of merger.merged.values()) {
    if (merged.session.eventCount === 0) count(skipped, "noEvents");
    else if (!hasQuestData(merged)) count(skipped, "noQuestData");
    else kept.push(merged);
  }

  for (const merged of kept) {
    const { episode, pendingGreetings } = merged.session;
    for (const pending of pendingGreetings) {
      resolveGreeting(episode.offers[pending.offer], pending, { locale: episode.locale, faction: episode.player.faction }, titles, greetings);
    }
  }

  const orderOf = sessionOrderTimes(kept);
  const characterInputs: CharacterInput[] = kept.map((merged) => {
    const { episode, startServerTime } = merged.session;
    return {
      episodeKey: episode.key,
      contributorId: merged.contributorId,
      player: episode.player,
      ...(startServerTime !== undefined ? { start: startServerTime } : {}),
      duration: episode.duration,
      order: orderOf.get(episode.key)!,
      ...(episode.sessionName ? { sessionName: episode.sessionName } : {}),
      firstLevel: episode.levels[0]?.v,
      lastLevel: episode.levels.at(-1)?.v,
      initialCompleted: episode.completed?.initial,
      finalCompleted: finalCompleted(episode),
    };
  });
  const characterOf = assignCharacters(characterInputs);
  const sessionOrderOf = orderSessions(characterInputs, characterOf);

  const episodes: QuestEpisode[] = kept.map((merged) => {
    const { key, ...draft } = merged.session.episode;
    return {
      key,
      contributorId: merged.contributorId,
      characterKey: characterOf.get(key)!,
      submissionIds: [...merged.submissionIds].sort(),
      sessionOrder: sessionOrderOf.get(key)!,
      ...draft,
    };
  });
  episodes.sort(
    (a, b) =>
      a.contributorId.localeCompare(b.contributorId) ||
      a.characterKey.localeCompare(b.characterKey) ||
      a.sessionOrder - b.sessionOrder ||
      a.key.localeCompare(b.key),
  );

  return { episodes, summary: summarize() };

  function summarize() {
    const builds = { forever: 0, other: 0, unknown: 0 };
    const completed = { modern: 0, legacy: 0, none: 0, earlyBulkAdds: 0 };
    const offers: Record<string, { total: number; listComplete: number }> = {};
    const questEvents: Partial<Record<QuestEventKind, number>> = {};
    const giverKinds: Record<string, number> = {};
    const distill = emptyDistillStats();
    let questEventsWithGiver = 0;
    let withQuestInfo = 0;
    let withPlayer = 0;
    for (const episode of episodes) {
      const version = episode.interfaceVersion;
      builds[version === undefined ? "unknown" : String(version).startsWith("16") ? "forever" : "other"]++;
      completed[episode.completed?.source ?? "none"]++;
      // A large add right after capture start means the history had not loaded yet at t = 0.
      if (episode.completed?.changes.some((change) => change.t < 30 && (change.add?.length ?? 0) > 20)) completed.earlyBulkAdds++;
      for (const offer of episode.offers) {
        const bucket = (offers[offer.source] ??= { total: 0, listComplete: 0 });
        bucket.total++;
        if (offer.listComplete) bucket.listComplete++;
      }
      for (const event of episode.questEvents) {
        count(questEvents, event.kind);
        if (event.giver) {
          questEventsWithGiver++;
          count(giverKinds, event.giver.kind);
        }
      }
      if (episode.questInfo) withQuestInfo++;
      if (episode.player.raceId && episode.player.classId) withPlayer++;
    }
    for (const merged of kept) addStats(distill, merged.session.stats);
    const totalOffers = Object.values(offers).reduce((sum, bucket) => sum + bucket.total, 0);
    const completeOffers = Object.values(offers).reduce((sum, bucket) => sum + bucket.listComplete, 0);
    const characters = new Set(episodes.map((episode) => episode.characterKey));

    return {
      params: { cacheVersion: CACHE_VERSION, distill: DISTILL_PARAMS, characters: CHARACTER_PARAMS },
      submissions: {
        total: submissions.length,
        decodedThisRun,
        failed: failures.length,
        failures: failures.slice(0, 20),
        workerCrashes: crashes.slice(0, 5),
        exports,
        withMultipleExports,
      },
      sessions: {
        copies: merger.copies,
        unique: merger.merged.size,
        duplicateCopiesMerged: merger.copies - merger.merged.size,
        seenUnderSeveralContributors: [...merger.merged.values()].filter((merged) => merged.contributorIds.size > 1).length,
        kept: episodes.length,
        skipped,
      },
      episodes: {
        builds,
        withPlayerContext: withPlayer,
        completed,
        withQuestInfo,
        contributors: new Set(episodes.map((episode) => episode.contributorId)).size,
        characters: characters.size,
      },
      offers: { bySource: offers, total: totalOffers, listComplete: completeOffers, listCompleteRatio: round(completeOffers / Math.max(1, totalOffers)) },
      greetings,
      questEvents: { byKind: questEvents, withGiver: questEventsWithGiver, giverKinds },
      distillCounts: distill,
    };
  }
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}
