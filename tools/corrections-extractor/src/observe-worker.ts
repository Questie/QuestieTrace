// Reads and observes submissions off the main thread (see extract-corrections.ts).

import { parentPort } from "worker_threads";
import { observeSubmission, type ObserveRequest } from "./observe-submission";

/** Session keys the main thread already folded in (see UsedKeysMessage). */
const usedKeys = new Set<string>();

parentPort!.on("message", (message: ObserveRequest | { usedKeys: string[] }) => {
  if ("usedKeys" in message) {
    for (const key of message.usedKeys) usedKeys.add(key);
    return;
  }
  parentPort!.postMessage(observeSubmission(message, usedKeys));
});
