// ============================================================
// Vite dev server middleware: serves trace data as JSON API
// ============================================================

import type { Connect, Plugin } from "vite";
import { basename, resolve } from "path";
import { existsSync, readFileSync } from "fs";
import { defaultTraceDataDir, listSubmissionFiles, readSubmission } from "../../../corrections-extractor/src/core/trace-data/submissions.js";
import type { TraceFile, SessionSummary, TraceFileSummary } from "../core/types.js";

const EXTRACT_ENTITY_TO_OUTPUT_FILE: Record<string, string> = {
  npc: "foreverNpcTraces.lua",
  quest: "foreverQuestTraces.lua",
  item: "foreverItemTraces.lua",
  object: "foreverObjectTraces.lua",
};

export function traceApiPlugin(): Plugin {
  let traceDataDir = "";
  let extractOutputDir = "";
  let traceFileNames: string[] = [];
  const traceFilePaths = new Map<string, string>();
  const loadedFiles = new Map<string, TraceFile>();
  const loadErrors = new Map<string, string>();

  function getOrLoad(fileName: string): TraceFile | null {
    if (loadedFiles.has(fileName)) return loadedFiles.get(fileName)!;

    const submission = readSubmission(traceFilePaths.get(fileName)!);
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

  function refreshFileList() {
    traceFilePaths.clear();
    try {
      for (const file of listSubmissionFiles(traceDataDir)) traceFilePaths.set(basename(file.path), file.path);
    } catch (e) {
      console.error(`[trace-api] ${e instanceof Error ? e.message : String(e)}`);
    }
    traceFileNames = [...traceFilePaths.keys()];
  }

  return {
    name: "trace-api",
    configureServer(server) {
      const root = server.config.root;
      traceDataDir = defaultTraceDataDir();
      extractOutputDir = resolve(root, "../corrections-extractor/output");
      refreshFileList();
      console.log(
        `[trace-api] Found ${traceFileNames.length} submission(s) in ${traceDataDir}`
      );

      const handleRequest = async (
        req: Connect.IncomingMessage,
        res: Parameters<Connect.NextHandleFunction>[1],
        next: Connect.NextFunction,
      ) => {
        if (!req.url?.startsWith("/api/")) return next();

        res.setHeader("Content-Type", "application/json");

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
          const data = getOrLoad(fileName);
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
          const data = getOrLoad(fileName);
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
          refreshFileList();
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
