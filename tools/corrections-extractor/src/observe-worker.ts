// Reads and observes submissions off the main thread (see extract-corrections.ts).

import { parentPort } from "worker_threads";
import { observeSubmission, type ObserveRequest } from "./observe-submission";

parentPort!.on("message", (request: ObserveRequest) => {
  parentPort!.postMessage(observeSubmission(request));
});
