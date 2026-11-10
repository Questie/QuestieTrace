import { afterEach, describe, expect, it, vi } from "vitest";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { TINY_EXPORT } from "../../../corrections-extractor/src/core/trace-data/fixtures";
import { traceApiPlugin } from "./middleware";

const tempDirs: string[] = [];

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

/** Creates `<tmp>/corrections-extractor/output` with `files`; returns the matching trace-analyzer root. */
function createExtractorOutput(files: Record<string, string>): string {
  const toolsDir = mkdtempSync(join(tmpdir(), "trace-analyzer-"));
  tempDirs.push(toolsDir);
  const outputDir = join(toolsDir, "corrections-extractor", "output");
  mkdirSync(outputDir, { recursive: true });
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(outputDir, name), content);
  }
  return join(toolsDir, "trace-analyzer");
}

type Request = { url?: string };
type Response = {
  statusCode: number;
  setHeader: ReturnType<typeof vi.fn>;
  end: ReturnType<typeof vi.fn>;
};
type Next = ReturnType<typeof vi.fn>;
type Middleware = (req: Request, res: Response, next: Next) => void;

/** Creates a trace-data checkout holding `files` (path under submissions/ -> content); returns its directory. */
function createTraceData(files: Record<string, string>): string {
  const traceDataDir = mkdtempSync(join(tmpdir(), "trace-data-"));
  tempDirs.push(traceDataDir);
  for (const [path, content] of Object.entries(files)) {
    const full = join(traceDataDir, "submissions", path);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, content);
  }
  return traceDataDir;
}

function setupHandler(root = "/__questie_trace_missing__/a/b", traceDataDir = "/__questie_trace_missing__/trace-data") {
  vi.stubEnv("TRACE_DATA_DIR", traceDataDir);
  vi.spyOn(console, "log").mockImplementation(() => undefined);
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  const use = vi.fn();
  const server = {
    config: { root },
    middlewares: { use },
  };
  const configureServer = traceApiPlugin().configureServer as (server: unknown) => void;
  configureServer(server);
  return use.mock.calls[0]?.[0] as Middleware;
}

/** Sends GET `url` to `handler` and waits for the response. */
function requester(handler: Middleware) {
  return async (url: string) => {
    const response = makeResponse();
    handler({ url }, response, vi.fn());
    // Generous: the bucket test starts real processes.
    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce(), { timeout: 5000 });
    return { status: response.statusCode, body: JSON.parse(response.end.mock.calls[0]?.[0]) };
  };
}

function makeResponse(): Response {
  return {
    statusCode: 200,
    setHeader: vi.fn(),
    end: vi.fn(),
  };
}

describe("traceApiPlugin middleware", () => {
  it("should forward malformed URI decoding errors to Connect", async () => {
    const handler = setupHandler();
    const response = makeResponse();
    const next = vi.fn();

    handler({ url: "/api/extract/%E0%A4%A" }, response, next);

    await vi.waitFor(() => expect(next).toHaveBeenCalledOnce());
    expect(next.mock.calls[0]?.[0]).toBeInstanceOf(URIError);
    expect(response.end).not.toHaveBeenCalled();
  });

  it("should respond 404 with a hint when no generated corrections exist", async () => {
    const handler = setupHandler();
    const response = makeResponse();

    handler({ url: "/api/extract/npc" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
    expect(response.end.mock.calls[0]?.[0]).toContain("npm run extract");
  });

  it("should serve generated corrections and meta from the corrections-extractor output", async () => {
    const root = createExtractorOutput({
      "foreverQuestTraces.lua": "function ForeverQuestTraces:Load()\nend",
      "meta.json": JSON.stringify({ sessionCount: 3, submissionCount: 2, skippedFiles: [{ name: "2026-10/bad.json", error: "boom" }], partialFiles: [{ name: "2026-10/partial.json", droppedExports: ["export 1 has no matching end marker"] }] }),
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/quest" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(JSON.parse(response.end.mock.calls[0]?.[0])).toEqual({
      fixes: "function ForeverQuestTraces:Load()\nend",
      sessionCount: 3,
      submissionCount: 2,
      skippedFiles: [{ name: "2026-10/bad.json", error: "boom" }],
      partialFiles: [{ name: "2026-10/partial.json", droppedExports: ["export 1 has no matching end marker"] }],
    });
  });

  it("should respond 500 with a JSON error when meta.json is malformed", async () => {
    const root = createExtractorOutput({
      "foreverQuestTraces.lua": "function ForeverQuestTraces:Load()\nend",
      "meta.json": "{ half-written",
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/quest" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(500);
    expect(JSON.parse(response.end.mock.calls[0]?.[0]).error).toContain("Unreadable meta.json");
  });

  it("should respond 404 for entity names inherited from Object.prototype", async () => {
    const handler = setupHandler();
    const response = makeResponse();

    handler({ url: "/api/extract/toString" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
    expect(response.end.mock.calls[0]?.[0]).toContain('Unknown extract entity \\"toString\\"');
  });

  it("should preserve normal API request handling", async () => {
    const handler = setupHandler();
    const response = makeResponse();
    const next = vi.fn();

    handler({ url: "/api/files" }, response, next);

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.setHeader).toHaveBeenCalledWith("Content-Type", "application/json");
    expect(response.end).toHaveBeenCalledWith("[]");
    expect(next).not.toHaveBeenCalled();
  });

  it("should list trace-data submissions and serve the sessions of every export in one", async () => {
    const traceDataDir = createTraceData({
      "2026-10/2026-10-01_000000Z_a.json": JSON.stringify({ id: "a", export_string: `${TINY_EXPORT}\n${TINY_EXPORT}` }),
      "2026-10/2026-10-02_000000Z_broken.json": JSON.stringify({ id: "broken", export_string: "not an export" }),
    });
    const request = requester(setupHandler(undefined, traceDataDir));

    expect((await request("/api/files/")).status).toBe(404);
    expect((await request("/api/file/2026-10-01_000000Z_a.json/sessions")).body).toHaveLength(2);
    expect((await request("/api/file/2026-10-01_000000Z_a.json/session/1")).body.startedAt).toBe(100.5);
    expect((await request("/api/file/2026-10-02_000000Z_broken.json/sessions")).status).toBe(500);
    expect((await request("/api/files")).body).toEqual([
      { name: "2026-10-01_000000Z_a.json", sessionCount: 2, error: null },
      { name: "2026-10-02_000000Z_broken.json", sessionCount: null, error: expect.stringMatching(/markers/) },
    ]);
  });

  it("should list and fetch submissions from the bucket when TRACE_DATA_DIR is not set", async () => {
    const bucketDir = createTraceData({ "2026-10/2026-10-01_000000Z_a.json": JSON.stringify({ id: "a", export_string: TINY_EXPORT }) });
    const binDir = mkdtempSync(join(tmpdir(), "fake-rclone-"));
    tempDirs.push(binDir);
    // Stands in for rclone: serves bucketDir for lsf and copy (see store/source.ts for the arguments).
    writeFileSync(
      join(binDir, "rclone"),
      `#!/bin/sh
case "$1" in
  lsf) cd "${bucketDir}/submissions" && find . -type f | sed 's|^./||' ;;
  copy) while read -r key; do mkdir -p "$(dirname "$9/$key")"; cp "${bucketDir}/submissions/$key" "$9/$key"; done < "$3" ;;
esac
`,
    );
    chmodSync(join(binDir, "rclone"), 0o755);
    vi.stubEnv("PATH", `${binDir}:${process.env.PATH}`);
    const request = requester(setupHandler(undefined, ""));

    expect((await request("/api/files")).body).toEqual([{ name: "2026-10-01_000000Z_a.json", sessionCount: null, error: null }]);
    expect((await request("/api/file/2026-10-01_000000Z_a.json/sessions")).body).toHaveLength(1);
  });
});
