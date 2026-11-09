// Where submissions come from: a local trace-data checkout, or the archive bucket through rclone.
//
// Keys are paths under submissions/, `<yyyy-mm>/<received stamp>_<submission id>.json`, so
// sorting them sorts submissions by receive time.

import { execFile } from "child_process";
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
      const { stdout } = await run(rclone, ["lsf", "--recursive", "--files-only", submissions], { maxBuffer: 1024 * 1024 * 1024 });
      return stdout.split("\n").filter((key) => key.endsWith(".json"));
    },
    fetch: async (keys) => {
      const dir = mkdtempSync(join(tmpdir(), "trace-batch-"));
      try {
        const list = join(dir, ".files");
        writeFileSync(list, keys.join("\n") + "\n");
        await run(rclone, ["copy", "--files-from-raw", list, "--transfers", "16", "--checkers", "16", submissions, dir], { maxBuffer: 64 * 1024 * 1024 });
        return dir;
      } catch (e) {
        rmSync(dir, { recursive: true, force: true });
        throw e;
      }
    },
    release: (dir) => rmSync(dir, { recursive: true, force: true }),
  };
}

/** TRACE_REMOTE (an rclone remote) when set, else the local checkout. */
export function sourceFromEnv(defaultTraceDataDir: string, env: NodeJS.ProcessEnv = process.env): SubmissionSource {
  const remote = env.TRACE_REMOTE;
  return remote && remote.length > 0 ? rcloneSource(remote) : localSource(defaultTraceDataDir);
}
