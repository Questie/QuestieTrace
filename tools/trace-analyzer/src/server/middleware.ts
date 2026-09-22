// ============================================================
// Vite dev server middleware: serves trace data as JSON API
// ============================================================

import type { Plugin } from "vite";
import { resolve } from "path";
import { readdirSync } from "fs";
import { loadTraceFile } from "../core/loader.js";
import type { TraceFile, SessionSummary, TraceFileSummary } from "../core/types.js";

export function traceApiPlugin(): Plugin {
  let traceDir = "";
  let traceFileNames: string[] = [];
  const loadedFiles = new Map<string, TraceFile>();
  const loadErrors = new Map<string, string>();

  function getOrLoad(fileName: string): TraceFile | null {
    if (loadedFiles.has(fileName)) return loadedFiles.get(fileName)!;

    const filePath = resolve(traceDir, fileName);
    try {
      const data = loadTraceFile(filePath);
      loadedFiles.set(fileName, data);
      loadErrors.delete(fileName);
      console.log(
        `[trace-api] Loaded ${data.sessions.length} session(s) from ${fileName}`
      );
      return data;
    } catch (e) {
      const err = String(e);
      loadErrors.set(fileName, err);
      console.error(`[trace-api] Failed to load ${fileName}: ${err}`);
      return null;
    }
  }

  function refreshFileList() {
    try {
      traceFileNames = readdirSync(traceDir)
        .filter((f) => f.endsWith(".lua"))
        .sort();
    } catch {
      traceFileNames = [];
      console.error(`[trace-api] Traces directory not found at ${traceDir}`);
    }
  }

  return {
    name: "trace-api",
    configureServer(server) {
      const root = server.config.root;
      traceDir = resolve(root, "../../Traces");
      refreshFileList();
      console.log(
        `[trace-api] Found ${traceFileNames.length} trace file(s) in ${traceDir}`
      );

      server.middlewares.use((req, res, next) => {
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
          const summaries: SessionSummary[] = data.sessions.map((s, index) => ({
            index,
            name: s.name ?? null,
            duration: s.duration ?? null,
            startedAt: s.startedAt,
            eventCount: s.events.length,
            functionCount: Object.keys(s.functions).length,
          }));
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
      });
    },
  };
}
