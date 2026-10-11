// Reading and writing the pipeline's cache files (paths in ./paths.ts).
//
// Episodes are streamed line by line so no consumer has to hold all of them at
// once; everything else is a single JSON document.

import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "fs";
import { dirname } from "path";
import { createInterface } from "readline";
import { candidatePath, paths } from "./paths";
import type { CandidateFile, GroundTruth, QuestCatalog, QuestEpisode } from "./types";

function requireFile(path: string, producedBy: string): void {
  if (!existsSync(path)) {
    throw new Error(`${path} does not exist. Run \`npm run relations -- ${producedBy}\` first.`);
  }
}

/** Writes via a temp file and rename, so readers never see a half-written file. */
export function writeJsonAtomic(path: string, value: unknown): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(value));
  renameSync(tmp, path);
}

export function readJson<T>(path: string, producedBy: string): T {
  requireFile(path, producedBy);
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

export async function* readEpisodes(path: string = paths.episodes): AsyncGenerator<QuestEpisode> {
  requireFile(path, "ingest");
  const lines = createInterface({ input: createReadStream(path, "utf8"), crlfDelay: Infinity });
  for await (const line of lines) {
    if (line.length > 0) yield JSON.parse(line) as QuestEpisode;
  }
}

/** Convenience for signals that need random access; fine at current data sizes. */
export async function loadEpisodes(path: string = paths.episodes): Promise<QuestEpisode[]> {
  const episodes: QuestEpisode[] = [];
  for await (const episode of readEpisodes(path)) episodes.push(episode);
  return episodes;
}

/** Writes episodes as JSON lines, atomically. */
export async function writeEpisodes(episodes: Iterable<QuestEpisode>, path: string = paths.episodes): Promise<void> {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp`;
  const out = createWriteStream(tmp, "utf8");
  // A stream error rejects whichever wait is pending, so a failed write can't hang the caller.
  let failure: Error | undefined;
  out.on("error", (err) => (failure = err));
  const waitFor = (event: "drain" | "finish") =>
    new Promise<void>((resolveWait, reject) => {
      if (failure) return reject(failure);
      const onError = (err: Error) => {
        out.off(event, onEvent);
        reject(err);
      };
      const onEvent = () => {
        out.off("error", onError);
        resolveWait();
      };
      out.once(event, onEvent);
      out.once("error", onError);
    });
  try {
    for (const episode of episodes) {
      if (!out.write(JSON.stringify(episode) + "\n")) await waitFor("drain");
    }
    const finished = waitFor("finish");
    out.end();
    await finished;
    renameSync(tmp, path);
  } catch (err) {
    out.destroy();
    try {
      rmSync(tmp, { force: true });
    } catch {
      // Best effort: the write error is the one worth reporting.
    }
    throw err;
  }
}

export function loadCatalog(path: string = paths.catalog): QuestCatalog {
  return readJson<QuestCatalog>(path, "catalog");
}

export function loadGroundTruth(path: string = paths.groundTruth): GroundTruth {
  return readJson<GroundTruth>(path, "catalog");
}

export function writeCandidates(file: CandidateFile): string {
  const path = candidatePath(file.signal);
  writeJsonAtomic(path, file);
  return path;
}

export function loadCandidates(signal: string): CandidateFile {
  return readJson<CandidateFile>(candidatePath(signal), signal);
}
