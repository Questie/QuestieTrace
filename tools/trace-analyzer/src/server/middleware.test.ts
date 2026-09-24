import { afterEach, describe, expect, it, vi } from "vitest";
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

function setupHandler(ssrLoadModule = vi.fn()) {
  vi.spyOn(console, "log").mockImplementation(() => undefined);
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  const use = vi.fn();
  const server = {
    config: { root: "/__questie_trace_missing__/a/b" },
    middlewares: { use },
    ssrLoadModule,
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

  it("should forward SSR module load rejections to Connect", async () => {
    const error = new Error("module load failed");
    const handler = setupHandler(vi.fn().mockRejectedValue(error));
    const response = makeResponse();
    const next = vi.fn();

    handler({ url: "/api/extract/npc" }, response, next);

    await vi.waitFor(() => expect(next).toHaveBeenCalledWith(error));
    expect(response.end).not.toHaveBeenCalled();
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
