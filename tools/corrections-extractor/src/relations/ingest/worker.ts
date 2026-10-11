// Worker thread for phase 1: decodes and distills one submission per message and writes its
// cache file, so only a small summary crosses back to the main thread.

import { parentPort } from "worker_threads";
import { writeJsonAtomic } from "../core/io";
import { cachePath, processSubmission } from "./submission";

export interface WorkerRequest {
  path: string;
  cacheId: string;
}

export interface WorkerReply {
  path: string;
  error?: string;
  /** Set when the worker itself failed (not a decode error, which is cached like any result). */
  crash?: string;
}

parentPort?.on("message", (request: WorkerRequest) => {
  try {
    const result = processSubmission(request.path);
    writeJsonAtomic(cachePath(request.cacheId), result);
    parentPort!.postMessage({ path: request.path, error: result.error } satisfies WorkerReply);
  } catch (e) {
    parentPort!.postMessage({ path: request.path, crash: e instanceof Error ? (e.stack ?? e.message) : String(e) } satisfies WorkerReply);
  }
});
