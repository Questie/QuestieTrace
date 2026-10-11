// ============================================================
// Vite dev server middleware: serves trace data as JSON API
// ============================================================

import type { Connect, Plugin } from "vite";
import { resolve } from "path";
import { existsSync, readdirSync, readFileSync } from "fs";
import { loadTraceFile } from "../core/loader.js";
import type { TraceFile, SessionSummary, TraceFileSummary } from "../core/types.js";

const EXTRACT_ENTITY_TO_OUTPUT_FILE: Record<string, string> = {
  npc: "foreverNpcTraces.lua",
  quest: "foreverQuestTraces.lua",
  item: "foreverItemTraces.lua",
  object: "foreverObjectTraces.lua",
};

// Quest relations come from a second pipeline (`npm run relations`) that publishes into the same
// output/ with its own meta file, so the two never overwrite each other.
const RELATIONS_LUA_FILE = "foreverQuestRelationTraces.lua";
const RELATIONS_META_FILE = "relations-meta.json";
const RELATIONS_REVIEW_FILE = "relations-review.md";
const RELATIONS_MISSING =
  "No generated quest relations found. Run `npm run relations -- pipeline` in tools/corrections-extractor first.";

export function traceApiPlugin(): Plugin {
  let traceDir = "";
  let extractOutputDir = "";
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
      extractOutputDir = resolve(root, "../corrections-extractor/output");
      refreshFileList();
      console.log(
        `[trace-api] Found ${traceFileNames.length} trace file(s) in ${traceDir}`
      );

      const handleRequest = async (
        req: Connect.IncomingMessage,
        res: Parameters<Connect.NextHandleFunction>[1],
        next: Connect.NextFunction,
      ) => {
        if (!req.url?.startsWith("/api/")) return next();
        // Routes match on the path alone, so a query string never sends a request to the wrong route.
        const pathname = req.url.split("?")[0];

        res.setHeader("Content-Type", "application/json");

        // GET /api/files — list available trace files
        if (pathname === "/api/files") {
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
        const sessionsMatch = pathname.match(
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
        const sessionMatch = pathname.match(
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

        // GET /api/extract/relations — the quest relations module and its relations-meta.json
        if (pathname === "/api/extract/relations") {
          const luaPath = resolve(extractOutputDir, RELATIONS_LUA_FILE);
          const metaPath = resolve(extractOutputDir, RELATIONS_META_FILE);
          if (!existsSync(luaPath) || !existsSync(metaPath)) {
            res.statusCode = 404;
            res.end(JSON.stringify({ error: RELATIONS_MISSING }));
            return;
          }

          let meta;
          try {
            meta = JSON.parse(readFileSync(metaPath, "utf8"));
          } catch (e) {
            res.statusCode = 500;
            res.end(
              JSON.stringify({
                error: `Unreadable ${RELATIONS_META_FILE} (${String(e)}). Run \`npm run relations -- combine\` in tools/corrections-extractor again.`,
              }),
            );
            return;
          }
          res.end(JSON.stringify({ fixes: readFileSync(luaPath, "utf8"), meta }));
          return;
        }

        // GET /api/extract/relations/review — the review report a person reads before trusting the module.
        // Like the module, only served with relations-meta.json, which combine removes while it rewrites both.
        if (pathname === "/api/extract/relations/review") {
          const reviewPath = resolve(extractOutputDir, RELATIONS_REVIEW_FILE);
          if (!existsSync(reviewPath) || !existsSync(resolve(extractOutputDir, RELATIONS_META_FILE))) {
            res.statusCode = 404;
            res.end(JSON.stringify({ error: RELATIONS_MISSING }));
            return;
          }
          res.end(JSON.stringify({ review: readFileSync(reviewPath, "utf8") }));
          return;
        }

        // GET /api/extract/{npc|quest|item|object} — corrections generated by tools/corrections-extractor
        const extractMatch = pathname.match(/^\/api\/extract\/([^/]+)$/);
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
              fileCount: meta.fileCount,
              skippedFiles: meta.skippedFiles,
            }),
          );
          return;
        }

        // GET /api/reload — refresh file list and clear cache
        if (pathname === "/api/reload") {
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
