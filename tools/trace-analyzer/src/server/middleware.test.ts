import { afterEach, describe, expect, it, vi } from "vitest";
import { mkdirSync, mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { traceApiPlugin } from "./middleware";

afterEach(() => {
  vi.restoreAllMocks();
});

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
    const toolsDir = mkdtempSync(join(tmpdir(), "trace-analyzer-"));
    const outputDir = join(toolsDir, "corrections-extractor", "output");
    mkdirSync(outputDir, { recursive: true });
    writeFileSync(join(outputDir, "foreverQuestTraces.lua"), "function ForeverQuestTraces:Load()\nend");
    writeFileSync(
      join(outputDir, "meta.json"),
      JSON.stringify({ sessionCount: 3, fileCount: 2, skippedFiles: [{ name: "bad.lua", error: "boom" }] }),
    );
    const handler = setupHandler(join(toolsDir, "trace-analyzer"));
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
