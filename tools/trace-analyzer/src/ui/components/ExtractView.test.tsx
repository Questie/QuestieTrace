import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ExtractView } from "./ExtractView";

interface MockResult {
  fixes: string;
  sessionCount: number;
  fileCount: number;
  skippedFiles: { name: string; error: string }[];
}

const RESULTS: Record<string, MockResult> = {
  npc: { fixes: "function ForeverTraceNpcFixes:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  quest: { fixes: "function ForeverTraceQuestFixes:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  item: { fixes: "function ForeverTraceItemFixes:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  object: { fixes: "function ForeverTraceObjectFixes:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
};

function mockSuccessFetch(overrides: Partial<Record<string, MockResult>> = {}) {
  const results = { ...RESULTS, ...overrides };
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      const entity = url.match(/\/api\/extract\/(\w+)/)?.[1];
      const body = entity ? results[entity] : { error: "unknown entity" };
      return Promise.resolve({ json: () => Promise.resolve(body) } as Response);
    }),
  );
}

describe("ExtractView", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should show a Generate button and no sections before anything is triggered", () => {
    render(<ExtractView />);

    expect(screen.getByText("Generate")).toBeInTheDocument();
    expect(screen.queryByText("NPC Fixes")).not.toBeInTheDocument();
  });

  it("should show a loading state while requests are in flight", () => {
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    expect(screen.getByText("Generating...")).toBeInTheDocument();
    expect(screen.getByText("Generating...")).toBeDisabled();
  });

  it("should render all four entity sections with their own Lua output on success", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText("NPC Fixes")).toBeInTheDocument());
    expect(screen.getByText("Quest Fixes")).toBeInTheDocument();
    expect(screen.getByText("Item Fixes")).toBeInTheDocument();
    expect(screen.getByText("Object Fixes")).toBeInTheDocument();
    expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument();
    expect(screen.getAllByText("Copy to clipboard")).toHaveLength(4);
    expect(screen.getByText(/ForeverTraceItemFixes:Load/)).toBeInTheDocument();
  });

  it("should list skipped files as a warning when some trace files fail to load", async () => {
    mockSuccessFetch({ npc: { ...RESULTS.npc, skippedFiles: [{ name: "bad.lua", error: "unexpected token" }] } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/Skipped 1 file/)).toBeInTheDocument());
    expect(screen.getByText(/bad\.lua: unexpected token/)).toBeInTheDocument();
  });

  it("should show an error message and allow retrying when a request fails", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network down"))));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/Error: network down/)).toBeInTheDocument());

    mockSuccessFetch();
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText("NPC Fixes")).toBeInTheDocument());
  });

  it("should trigger a download with the correct filename for a given section", async () => {
    mockSuccessFetch();
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));
    await waitFor(() => expect(screen.getByText("Quest Fixes")).toBeInTheDocument());

    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const createElementSpy = vi.spyOn(document, "createElement");

    fireEvent.click(screen.getByText("Download ForeverTraceQuestFixes.lua"));

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("ForeverTraceQuestFixes.lua");
    expect(clickSpy).toHaveBeenCalled();
  });
});
