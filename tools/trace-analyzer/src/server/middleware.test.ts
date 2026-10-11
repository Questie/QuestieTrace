import { afterEach, describe, expect, it, vi } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { traceApiPlugin } from "./middleware";

const tempDirs: string[] = [];

afterEach(() => {
  vi.restoreAllMocks();
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

function setupHandler(root = "/__questie_trace_missing__/a/b") {
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
      "meta.json": JSON.stringify({ sessionCount: 3, fileCount: 2, skippedFiles: [{ name: "bad.lua", error: "boom" }] }),
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/quest" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(JSON.parse(response.end.mock.calls[0]?.[0])).toEqual({
      fixes: "function ForeverQuestTraces:Load()\nend",
      sessionCount: 3,
      fileCount: 2,
      skippedFiles: [{ name: "bad.lua", error: "boom" }],
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

  it("should serve the quest relations module with its own meta file", async () => {
    const root = createExtractorOutput({
      "foreverQuestRelationTraces.lua": "function ForeverQuestRelationTraces:Load()\nend",
      "relations-meta.json": JSON.stringify({ minScore: 0.9, lua: { quests: 2, relations: 3 } }),
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/relations" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(JSON.parse(response.end.mock.calls[0]?.[0])).toEqual({
      fixes: "function ForeverQuestRelationTraces:Load()\nend",
      meta: { minScore: 0.9, lua: { quests: 2, relations: 3 } },
    });
  });

  it("should respond 404 with the relations hint until a combine run has written relations-meta.json", async () => {
    // The field extractor's meta.json says nothing about the relations module next to it.
    const root = createExtractorOutput({
      "foreverQuestRelationTraces.lua": "function ForeverQuestRelationTraces:Load()\nend",
      "meta.json": JSON.stringify({ sessionCount: 3, fileCount: 2, skippedFiles: [] }),
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/relations" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
    expect(response.end.mock.calls[0]?.[0]).toContain("npm run relations -- pipeline");
  });

  it("should respond 404 when relations-meta.json exists but the module is missing", async () => {
    const root = createExtractorOutput({ "relations-meta.json": JSON.stringify({ minScore: 0.9 }) });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/relations" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
    expect(response.end.mock.calls[0]?.[0]).toContain("npm run relations -- pipeline");
  });

  it("should serve the relations review report, ignoring a query string", async () => {
    const root = createExtractorOutput({
      "relations-review.md": "# Quest relation review\n",
      "relations-meta.json": JSON.stringify({ minScore: 0.9 }),
    });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/relations/review?t=1" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(JSON.parse(response.end.mock.calls[0]?.[0])).toEqual({ review: "# Quest relation review\n" });
  });

  it("should not serve a review left behind while combine has relations-meta.json removed", async () => {
    const root = createExtractorOutput({ "relations-review.md": "# Quest relation review\n" });
    const handler = setupHandler(root);
    const response = makeResponse();

    handler({ url: "/api/extract/relations/review" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
  });

  it("should respond 404 for entity names inherited from Object.prototype", async () => {
    const handler = setupHandler();
    const response = makeResponse();

    handler({ url: "/api/extract/toString" }, response, vi.fn());

    await vi.waitFor(() => expect(response.end).toHaveBeenCalledOnce());
    expect(response.statusCode).toBe(404);
    expect(response.end.mock.calls[0]?.[0]).toContain('Unknown extract entity \\"toString\\"');
  });

  it("should preserve normal API request handling", () => {
    const handler = setupHandler();
    const response = makeResponse();
    const next = vi.fn();

    handler({ url: "/api/files" }, response, next);

    expect(response.setHeader).toHaveBeenCalledWith("Content-Type", "application/json");
    expect(response.end).toHaveBeenCalledWith("[]");
    expect(next).not.toHaveBeenCalled();
  });
});
