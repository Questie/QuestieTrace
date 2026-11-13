// Where submissions come from: a local trace-data checkout, or the archive bucket through rclone.
//
// Keys are paths under submissions/, `<yyyy-mm>/<received stamp>_<submission id>.json`, so
// sorting them sorts submissions by receive time.

import { execFile } from "child_process";
import { createHash } from "crypto";
import { mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, relative, resolve } from "path";
import { promisify } from "util";
import { listSubmissionFiles } from "../core/trace-data/submissions";

const run = promisify(execFile);

export interface SubmissionSource {
  /** Every submission key, in any order. */
  list(): Promise<string[]>;
  /** Makes `keys` readable locally. Returns their directory: `<dir>/<key>`; pass it to release. */
  fetch(keys: string[]): Promise<string>;
  /** Frees what fetch made local. */
  release(dir: string): void;
}

/** A trace-data checkout: `<traceDataDir>/submissions/<key>`. Nothing is copied. */
export function localSource(traceDataDir: string): SubmissionSource {
  const submissionsDir = resolve(traceDataDir, "submissions");
  return {
    list: async () => listSubmissionFiles(traceDataDir).map((file) => relative(submissionsDir, file.path).split("\\").join("/")),
    fetch: async () => submissionsDir,
    release: () => undefined,
  };
}

/**
 * An rclone remote holding the archive, e.g. `ovhcloud-de:trace-intake-ovh`: `<remote>/submissions/<key>`.
 * Fetched keys are copied into a temporary directory, which release removes.
 */
export function rcloneSource(remote: string, rclone = "rclone"): SubmissionSource {
  const submissions = `${remote.replace(/\/+$/, "")}/submissions`;
  return {
    list: async () => {
      let stdout: string;
      try {
        ({ stdout } = await run(rclone, ["lsf", "--recursive", "--files-only", "--fast-list", submissions], { maxBuffer: 1024 * 1024 * 1024 }));
      } catch (e) {
        const reason = e instanceof Error ? e.message.trim() : String(e);
        throw new Error(`Listing ${submissions} with rclone failed. Set TRACE_REMOTE to your rclone remote, or TRACE_DATA_DIR to a local trace-data checkout.\n${reason}`);
      }
      return stdout.split("\n").filter((key) => key.endsWith(".json"));
    },
    fetch: async (keys) => {
      const dir = mkdtempSync(join(tmpdir(), "trace-batch-"));
      try {
        const list = join(dir, ".files");
        writeFileSync(list, keys.join("\n") + "\n");
        await run(rclone, ["copy", "--files-from-raw", list, "--transfers", "64", "--checkers", "64", submissions, dir], { maxBuffer: 64 * 1024 * 1024 });
        return dir;
      } catch (e) {
        rmSync(dir, { recursive: true, force: true });
        throw e;
      }
    },
    release: (dir) => rmSync(dir, { recursive: true, force: true }),
  };
}

/** The team's archive bucket, as named in everyone's rclone config. */
export const DEFAULT_REMOTE = "ovhcloud-de:trace-intake-ovh";

/** TRACE_DATA_DIR (a local trace-data checkout) when set, else TRACE_REMOTE, else DEFAULT_REMOTE. */
export function sourceFromEnv(env: NodeJS.ProcessEnv = process.env): SubmissionSource {
  if (env.TRACE_DATA_DIR) return localSource(resolve(env.TRACE_DATA_DIR));
  return rcloneSource(env.TRACE_REMOTE || DEFAULT_REMOTE);
}

/**
 * The `percent` of `source`'s submissions whose key hashes below it: the same ones every run,
 * and a sample grows by adding submissions (never swapping them) as `percent` rises.
 */
export function sampledSource(source: SubmissionSource, percent: number): SubmissionSource {
  const inSample = (key: string) => createHash("sha256").update(key).digest().readUInt32BE(0) / 2 ** 32 < percent / 100;
  return { ...source, list: async () => (await source.list()).filter(inSample) };
}

/** EXTRACT_SAMPLE as a percentage of submissions, or undefined for all of them. */
export function samplePercentFromEnv(env: NodeJS.ProcessEnv = process.env): number | undefined {
  const value = env.EXTRACT_SAMPLE;
  if (value === undefined || value === "") return undefined;
  const percent = Number(value);
  if (!/^\d+(\.\d+)?$/.test(value) || percent <= 0 || percent > 100) throw new Error(`EXTRACT_SAMPLE must be a percentage above 0 and at most 100 (got "${value}")`);
  return percent;
}
