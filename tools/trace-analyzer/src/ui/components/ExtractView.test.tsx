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

  it("should show a 'Generate all' button before generation", () => {
    render(<ExtractView />);

    expect(screen.getByText("Generate all")).toBeInTheDocument();
    expect(screen.queryByText("3 session(s)")).not.toBeInTheDocument();
  });

  it("should show tabs in idle state (before any generation)", () => {
    render(<ExtractView />);

    expect(screen.getByText("NPC")).toBeInTheDocument();
    expect(screen.getByText("Quest")).toBeInTheDocument();
    expect(screen.getByText("Item")).toBeInTheDocument();
    expect(screen.getByText("Object")).toBeInTheDocument();
  });

  it("should show placeholder text and allow per-entity Generate in idle state", () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Item"));

    expect(screen.getByText(/Click "Generate all"/)).toBeInTheDocument();

    // The per-entity Generate button should be clickable in idle state
    const generateButtons = screen.getAllByText("Generate");
    const itemTabGenerate = generateButtons[generateButtons.length - 1];
    fireEvent.click(itemTabGenerate);

    waitFor(() => {
      expect(screen.getByText(/ForeverTraceItemFixes:Load/)).toBeInTheDocument();
    });
  });

  it("should show a loading state while 'Generate all' requests are in flight", () => {
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    expect(screen.getByText("Generating all...")).toBeInTheDocument();
    expect(screen.getByText("Generating all...")).toBeDisabled();
  });

  it("should render tabs for all four entities after 'Generate all'", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument());
    expect(screen.getByText("NPC")).toBeInTheDocument();
    expect(screen.getByText("Quest")).toBeInTheDocument();
    expect(screen.getByText("Item")).toBeInTheDocument();
    expect(screen.getByText("Object")).toBeInTheDocument();
  });

  it("should show the NPC tab content by default after generation", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText(/ForeverTraceNpcFixes:Load/)).toBeInTheDocument());
  });

  it("should switch tabs when clicking on a different entity tab", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText("Quest")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Quest"));

    await waitFor(() => expect(screen.getByText(/ForeverTraceQuestFixes:Load/)).toBeInTheDocument());
    expect(screen.queryByText(/ForeverTraceNpcFixes:Load/)).not.toBeInTheDocument();
  });

  it("should allow per-entity 'Generate' button to regenerate just one entity", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText(/ForeverTraceNpcFixes:Load/)).toBeInTheDocument());

    // Switch to Item tab
    fireEvent.click(screen.getByText("Item"));
    await waitFor(() => expect(screen.getByText(/ForeverTraceItemFixes:Load/)).toBeInTheDocument());

    // Click the per-entity Generate button (in the Item tab context)
    const generateButtons = screen.getAllByText("Generate");
    const itemTabGenerate = generateButtons[generateButtons.length - 1];
    fireEvent.click(itemTabGenerate);

    await waitFor(() => expect(screen.getByText(/ForeverTraceItemFixes:Load/)).toBeInTheDocument());
  });

  it("should list skipped files as a warning when some trace files fail to load", async () => {
    mockSuccessFetch({ npc: { ...RESULTS.npc, skippedFiles: [{ name: "bad.lua", error: "unexpected token" }] } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText(/Skipped 1 file/)).toBeInTheDocument());
    expect(screen.getByText(/bad\.lua: unexpected token/)).toBeInTheDocument();
  });

  it("should show an error message and allow retrying when a request fails", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network down"))));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText(/Error: network down/)).toBeInTheDocument());

    mockSuccessFetch();
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument());
  });

  it("should show placeholder text when a tab has no data", async () => {
    mockSuccessFetch({ object: { ...RESULTS.object, fixes: "" } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText("Object")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Object"));

    await waitFor(() => expect(screen.getByText(/No data — click Generate/)).toBeInTheDocument());
  });

  it("should trigger a download with the correct filename from the active tab", async () => {
    mockSuccessFetch();
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));
    await waitFor(() => expect(screen.getByText("Quest")).toBeInTheDocument());

    fireEvent.click(screen.getByText("Quest"));
    await waitFor(() => expect(screen.getByText(/ForeverTraceQuestFixes:Load/)).toBeInTheDocument());

    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const createElementSpy = vi.spyOn(document, "createElement");

    fireEvent.click(screen.getByText("Download"));

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("ForeverTraceQuestFixes.lua");
    expect(clickSpy).toHaveBeenCalled();
  });

  it("should disable copy/download buttons when tab has no data", async () => {
    mockSuccessFetch({ item: { ...RESULTS.item, fixes: "" } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate all"));

    await waitFor(() => expect(screen.getByText("Item")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Item"));

    const buttons = screen.getAllByRole("button");
    const copyButton = buttons.find((b) => b.textContent === "Copy to clipboard");
    const downloadButton = buttons.find((b) => b.textContent === "Download");

    expect(copyButton).toBeDisabled();
    expect(downloadButton).toBeDisabled();
  });
});
