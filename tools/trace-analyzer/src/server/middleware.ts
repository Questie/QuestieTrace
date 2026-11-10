// ============================================================
// Vite dev server middleware: serves trace data as JSON API
// ============================================================

import type { Connect, Plugin } from "vite";
import { basename, join, resolve } from "path";
import { existsSync, readFileSync } from "fs";
import { readSubmission } from "../../../corrections-extractor/src/core/trace-data/submissions.js";
import { DEFAULT_REMOTE, sourceFromEnv, type SubmissionSource } from "../../../corrections-extractor/src/store/source.js";
import type { TraceFile, SessionSummary, TraceFileSummary } from "../core/types.js";

const EXTRACT_ENTITY_TO_OUTPUT_FILE: Record<string, string> = {
  npc: "foreverNpcTraces.lua",
  quest: "foreverQuestTraces.lua",
  item: "foreverItemTraces.lua",
  object: "foreverObjectTraces.lua",
};

/** Where submissions come from, for log messages (see sourceFromEnv). */
function sourceName(env: NodeJS.ProcessEnv = process.env): string {
  return env.TRACE_DATA_DIR || env.TRACE_REMOTE || DEFAULT_REMOTE;
}

export function traceApiPlugin(): Plugin {
  let source: SubmissionSource;
  let extractOutputDir = "";
  let traceFileNames: string[] = [];
  /** File name -> source key (`<yyyy-mm>/<file name>`). Names hold the receive stamp and id, so they are unique. */
  let traceFileKeys = new Map<string, string>();
  const loadedFiles = new Map<string, TraceFile>();
  const loadErrors = new Map<string, string>();
  /** Loads in progress, so concurrent requests for one file fetch it once. */
  const loading = new Map<string, Promise<TraceFile | null>>();
  let listing: Promise<void> = Promise.resolve();
  /** Increased by every reload, so loads started before one do not store their result. */
  let generation = 0;

  async function getOrLoad(fileName: string): Promise<TraceFile | null> {
    if (loadedFiles.has(fileName)) return loadedFiles.get(fileName)!;
    let pending = loading.get(fileName);
    if (!pending) {
      const started: Promise<TraceFile | null> = load(fileName).finally(() => {
        // A reload may have started another load of this file meanwhile.
        if (loading.get(fileName) === started) loading.delete(fileName);
      });
      pending = started;
      loading.set(fileName, started);
    }
    return pending;
  }

  async function load(fileName: string): Promise<TraceFile | null> {
    const key = traceFileKeys.get(fileName)!;
    const startedIn = generation;
    let submission;
    try {
      // Downloads the file from the bucket; local checkouts are read in place.
      const dir = await source.fetch([key]);
      try {
        submission = readSubmission(join(dir, key));
      } finally {
        source.release(dir);
      }
    } catch (e) {
      if (startedIn !== generation) return reloadedAgain(fileName);
      const error = e instanceof Error ? e.message : String(e);
      loadErrors.set(fileName, error);
      console.error(`[trace-api] Failed to fetch ${fileName}: ${error}`);
      return null;
    }
    if (startedIn !== generation) return reloadedAgain(fileName);
    if (submission.error) {
      loadErrors.set(fileName, submission.error);
      console.error(`[trace-api] Failed to load ${fileName}: ${submission.error}`);
      return null;
    }
    if (submission.droppedExports.length > 0) {
      console.warn(`[trace-api] Dropped exports of ${fileName}: ${submission.droppedExports.join("; ")}`);
    }
    // A submission can hold several exports; their sessions are listed one after another.
    const data = { sessions: submission.traces.flatMap((trace) => trace.sessions) } as TraceFile;
    loadedFiles.set(fileName, data);
    loadErrors.delete(fileName);
    console.log(
      `[trace-api] Loaded ${data.sessions.length} session(s) from ${fileName}`
    );
    return data;
  }

  /** A load overtaken by a reload: load the file again from the new listing, if it still has it. */
  async function reloadedAgain(fileName: string): Promise<TraceFile | null> {
    await listing;
    return traceFileKeys.has(fileName) ? getOrLoad(fileName) : null;
  }

  async function refreshFileList(): Promise<void> {
    // The new list replaces the old one at once, so requests during a listing still see a complete one.
    const keys = new Map<string, string>();
    try {
      for (const key of (await source.list()).sort()) keys.set(basename(key), key);
    } catch (e) {
      console.error(`[trace-api] ${e instanceof Error ? e.message : String(e)}`);
    }
    traceFileKeys = keys;
    traceFileNames = [...keys.keys()];
  }

  return {
    name: "trace-api",
    configureServer(server) {
      const root = server.config.root;
      source = sourceFromEnv();
      extractOutputDir = resolve(root, "../corrections-extractor/output");
      listing = refreshFileList().then(() => console.log(`[trace-api] Found ${traceFileNames.length} submission(s) in ${sourceName()}`));

      const handleRequest = async (
        req: Connect.IncomingMessage,
        res: Parameters<Connect.NextHandleFunction>[1],
        next: Connect.NextFunction,
      ) => {
        if (!req.url?.startsWith("/api/")) return next();

        res.setHeader("Content-Type", "application/json");
        // Only the extract output does not need the submission list.
        if (!req.url.startsWith("/api/extract/")) await listing;

        // GET /api/files — list available trace files
        if (req.url === "/api/files") {
          const summaries: TraceFileSummary[] = traceFileNames.map((name) => {
            const loaded = loadedFiles.get(name);
            return {
              name,
              sessionCount: loaded?.sessions.length ?? null,
              error: loadErrors.get(name) ?? null,
            };
          });
          res.end(JSON.stringify(summaries));
          return;
        }

        // GET /api/file/:filename/sessions
        const sessionsMatch = req.url.match(
          /^\/api\/file\/([^/]+)\/sessions$/
        );
        if (sessionsMatch) {
          const fileName = decodeURIComponent(sessionsMatch[1]);
          if (!traceFileNames.includes(fileName)) {
            res.statusCode = 404;
            res.end(JSON.stringify({ error: `File "${fileName}" not found` }));
            return;
          }
          const data = await getOrLoad(fileName);
          if (!data) {
            res.statusCode = 500;
            res.end(
              JSON.stringify({ error: loadErrors.get(fileName) ?? "Unknown error" })
            );
            return;
          }
          let summaries: SessionSummary[];
          try {
            summaries = data.sessions.map((s, index) => ({
              index,
              name: s.name ?? null,
              duration: s.duration ?? null,
              startedAt: s.startedAt,
              eventCount: s.events.length,
              functionCount: Object.keys(s.functions).length,
            }));
          } catch (e) {
            res.statusCode = 500;
            res.end(JSON.stringify({ error: `Could not summarize the sessions of ${fileName}: ${e instanceof Error ? e.name : typeof e}` }));
            return;
          }
          res.end(JSON.stringify(summaries));
          return;
        }

        // GET /api/file/:filename/session/:index
        const sessionMatch = req.url.match(
          /^\/api\/file\/([^/]+)\/session\/(\d+)$/
        );
        if (sessionMatch) {
          const fileName = decodeURIComponent(sessionMatch[1]);
          const sessionIndex = Number(sessionMatch[2]);
          if (!traceFileNames.includes(fileName)) {
            res.statusCode = 404;
            res.end(JSON.stringify({ error: `File "${fileName}" not found` }));
            return;
          }
          const data = await getOrLoad(fileName);
          if (!data) {
            res.statusCode = 500;
            res.end(
              JSON.stringify({ error: loadErrors.get(fileName) ?? "Unknown error" })
            );
            return;
          }
          const session = data.sessions[sessionIndex];
          if (!session) {
            res.statusCode = 404;
            res.end(
              JSON.stringify({
                error: `Session ${sessionIndex} not found in ${fileName}`,
              })
            );
            return;
          }
          res.end(JSON.stringify(session));
          return;
        }

        // GET /api/extract/{npc|quest|item|object} — corrections generated by tools/corrections-extractor
        const extractMatch = req.url.match(/^\/api\/extract\/([^/]+)$/);
        if (extractMatch) {
          const entity = decodeURIComponent(extractMatch[1]);
          if (!Object.hasOwn(EXTRACT_ENTITY_TO_OUTPUT_FILE, entity)) {
            res.statusCode = 404;
            res.end(JSON.stringify({ error: `Unknown extract entity "${entity}"` }));
            return;
          }

          const fixesPath = resolve(extractOutputDir, EXTRACT_ENTITY_TO_OUTPUT_FILE[entity]);
          const metaPath = resolve(extractOutputDir, "meta.json");
          if (!existsSync(fixesPath) || !existsSync(metaPath)) {
            res.statusCode = 404;
            res.end(
              JSON.stringify({
                error: "No generated corrections found. Run `npm run extract` in tools/corrections-extractor first.",
              }),
            );
            return;
          }

          let meta;
          try {
            meta = JSON.parse(readFileSync(metaPath, "utf8"));
          } catch (e) {
            res.statusCode = 500;
            res.end(
              JSON.stringify({
                error: `Unreadable meta.json (${String(e)}). Run \`npm run extract\` in tools/corrections-extractor again.`,
              }),
            );
            return;
          }
          res.end(
            JSON.stringify({
              fixes: readFileSync(fixesPath, "utf8"),
              sessionCount: meta.sessionCount,
              submissionCount: meta.submissionCount,
              skippedFiles: meta.skippedFiles,
              partialFiles: meta.partialFiles ?? [],
            }),
          );
          return;
        }

        // GET /api/reload — refresh file list and clear cache
        if (req.url === "/api/reload") {
          loadedFiles.clear();
          loadErrors.clear();
          loading.clear();
          generation++;
          listing = refreshFileList();
          await listing;
          res.end(
            JSON.stringify({ ok: true, files: traceFileNames.length })
          );
          return;
        }

        res.statusCode = 404;
        res.end(JSON.stringify({ error: "Not found" }));
      };

      server.middlewares.use((req, res, next) => {
        void handleRequest(req, res, next).catch(next);
      });
    },
  };
}
