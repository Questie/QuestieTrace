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
  npc: { fixes: "function ForeverNpcTraces:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  quest: { fixes: "function ForeverQuestTraces:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  item: { fixes: "function ForeverItemTraces:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
  object: { fixes: "function ForeverObjectTraces:Load()\nend", sessionCount: 3, fileCount: 2, skippedFiles: [] },
};

const RELATIONS = {
  fixes: "function ForeverQuestRelationTraces:Load()\nend",
  meta: {
    generatedAt: "2026-10-11T00:00:00.000Z",
    minScore: 0.9,
    episodeCount: 120,
    characterCount: 40,
    lua: { quests: 2, relations: 3 },
    authored: { agree: 5, new: 1, conflict: 0 },
    caveat: "Held-out metrics are optimistic.",
  },
};

const REVIEW = { review: "# Quest relation review" };

/** Serves RESULTS per entity plus the relations module and review; `bodies` replaces any URL's body. */
function mockSuccessFetch(overrides: Partial<Record<string, MockResult>> = {}, bodies: Record<string, unknown> = {}) {
  const results = { ...RESULTS, ...overrides };
  const responses: Record<string, unknown> = {
    ...Object.fromEntries(Object.entries(results).map(([entity, body]) => [`/api/extract/${entity}`, body])),
    "/api/extract/relations": RELATIONS,
    "/api/extract/relations/review": REVIEW,
    ...bodies,
  };
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      const body = responses[url] ?? { error: `unknown url ${url}` };
      return Promise.resolve({ json: () => Promise.resolve(body) } as Response);
    }),
  );
}

describe("ExtractView", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should show a 'Load all' button before loading", () => {
    render(<ExtractView />);

    expect(screen.getByText("Load all")).toBeInTheDocument();
    expect(screen.queryByText("3 session(s)")).not.toBeInTheDocument();
  });

  it("should show tabs in idle state (before any generation)", () => {
    render(<ExtractView />);

    expect(screen.getByText("NPC")).toBeInTheDocument();
    expect(screen.getByText("Quest")).toBeInTheDocument();
    expect(screen.getByText("Item")).toBeInTheDocument();
    expect(screen.getByText("Object")).toBeInTheDocument();
  });

  it("should show placeholder text and allow per-entity Reload in idle state", () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Item"));

    expect(screen.getByText(/Run `npm run extract`/)).toBeInTheDocument();

    // The per-entity Reload button should be clickable in idle state
    const reloadButtons = screen.getAllByText("Reload");
    const itemTabReload = reloadButtons[reloadButtons.length - 1];
    fireEvent.click(itemTabReload);

    waitFor(() => {
      expect(screen.getByText(/ForeverItemTraces:Load/)).toBeInTheDocument();
    });
  });

  it("should show a loading state while 'Load all' requests are in flight", () => {
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    expect(screen.getByText("Loading all...")).toBeInTheDocument();
    expect(screen.getByText("Loading all...")).toBeDisabled();
  });

  it("should render tabs for all four entities after 'Load all'", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument());
    expect(screen.getByText("NPC")).toBeInTheDocument();
    expect(screen.getByText("Quest")).toBeInTheDocument();
    expect(screen.getByText("Item")).toBeInTheDocument();
    expect(screen.getByText("Object")).toBeInTheDocument();
  });

  it("should show the NPC tab content by default after generation", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText(/ForeverNpcTraces:Load/)).toBeInTheDocument());
  });

  it("should switch tabs when clicking on a different entity tab", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("Quest")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Quest"));

    await waitFor(() => expect(screen.getByText(/ForeverQuestTraces:Load/)).toBeInTheDocument());
    expect(screen.queryByText(/ForeverNpcTraces:Load/)).not.toBeInTheDocument();
  });

  it("should allow per-entity 'Reload' button to reload just one entity", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText(/ForeverNpcTraces:Load/)).toBeInTheDocument());

    // Switch to Item tab
    fireEvent.click(screen.getByText("Item"));
    await waitFor(() => expect(screen.getByText(/ForeverItemTraces:Load/)).toBeInTheDocument());

    // Click the per-entity Reload button (in the Item tab context)
    const reloadButtons = screen.getAllByText("Reload");
    const itemTabReload = reloadButtons[reloadButtons.length - 1];
    fireEvent.click(itemTabReload);

    await waitFor(() => expect(screen.getByText(/ForeverItemTraces:Load/)).toBeInTheDocument());
  });

  it("should list skipped files as a warning when some trace files fail to load", async () => {
    mockSuccessFetch({ npc: { ...RESULTS.npc, skippedFiles: [{ name: "bad.lua", error: "unexpected token" }] } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText(/Skipped 1 file/)).toBeInTheDocument());
    expect(screen.getByText(/bad\.lua: unexpected token/)).toBeInTheDocument();
  });

  it("should show an error message and allow retrying when a request fails", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network down"))));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText(/Error: network down/)).toBeInTheDocument());

    mockSuccessFetch();
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument());
  });

  it("should show placeholder text when a tab has no data", async () => {
    mockSuccessFetch({ object: { ...RESULTS.object, fixes: "" } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("Object")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Object"));

    await waitFor(() => expect(screen.getByText(/No data for this entity/)).toBeInTheDocument());
  });

  it("should trigger a download with the correct filename from the active tab", async () => {
    mockSuccessFetch();
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));
    await waitFor(() => expect(screen.getByText("Quest")).toBeInTheDocument());

    fireEvent.click(screen.getByText("Quest"));
    await waitFor(() => expect(screen.getByText(/ForeverQuestTraces:Load/)).toBeInTheDocument());

    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const createElementSpy = vi.spyOn(document, "createElement");

    fireEvent.click(screen.getByText("Download"));

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("foreverQuestTraces.lua");
    expect(clickSpy).toHaveBeenCalled();
  });

  it("should disable copy/download buttons when tab has no data", async () => {
    mockSuccessFetch({ item: { ...RESULTS.item, fixes: "" } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("Item")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Item"));

    const buttons = screen.getAllByRole("button");
    const copyButton = buttons.find((b) => b.textContent === "Copy to clipboard");
    const downloadButton = buttons.find((b) => b.textContent === "Download");

    expect(copyButton).toBeDisabled();
    expect(downloadButton).toBeDisabled();
  });

  it("should show the relations module with its summary and caveat in the Quest relations tab", async () => {
    mockSuccessFetch();

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));
    fireEvent.click(screen.getByText("Quest relations"));

    await waitFor(() => expect(screen.getByText(/ForeverQuestRelationTraces:Load/)).toBeInTheDocument());
    expect(screen.getByText(/2 Forever quest\(s\), 3 relation\(s\) from 120 episode\(s\) of 40 character\(s\)/)).toBeInTheDocument();
    expect(screen.getByText("Held-out metrics are optimistic.")).toBeInTheDocument();
  });

  it("should show the review report and download it under its own name", async () => {
    mockSuccessFetch();
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));
    fireEvent.click(screen.getByText("Quest relations"));
    await waitFor(() => expect(screen.getByText("Show review report")).toBeInTheDocument());

    fireEvent.click(screen.getByText("Show review report"));
    expect(screen.getByText("# Quest relation review")).toBeInTheDocument();

    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const createElementSpy = vi.spyOn(document, "createElement");
    fireEvent.click(screen.getByText("Download"));

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("relations-review.md");
  });

  it("should keep the field corrections loading when the relations pipeline has not run", async () => {
    const missing = { error: "No generated quest relations found. Run `npm run relations -- pipeline` in tools/corrections-extractor first." };
    mockSuccessFetch({}, { "/api/extract/relations": missing, "/api/extract/relations/review": missing });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));

    await waitFor(() => expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Quest relations"));
    await waitFor(() =>
      expect(screen.getByText(/Run `npm run relations -- pipeline` in tools\/corrections-extractor first/)).toBeInTheDocument(),
    );
  });

  it("should show a fallback for a relations meta file of another shape and keep the field tabs working", async () => {
    const { authored: _authored, ...metaWithoutAuthored } = RELATIONS.meta;
    mockSuccessFetch({}, { "/api/extract/relations": { ...RELATIONS, meta: metaWithoutAuthored } });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));
    fireEvent.click(screen.getByText("Quest relations"));

    await waitFor(() => expect(screen.getByText(/relations-meta.json has an unexpected shape/)).toBeInTheDocument());
    fireEvent.click(screen.getByText("NPC"));
    expect(screen.getByText(/ForeverNpcTraces:Load/)).toBeInTheDocument();
  });

  it("should contain a render error in the relations tab", async () => {
    // A module body that is not a string makes React throw while rendering the <pre>.
    mockSuccessFetch({}, { "/api/extract/relations": { ...RELATIONS, fixes: { unexpected: true } } });
    vi.spyOn(console, "error").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Load all"));
    fireEvent.click(screen.getByText("Quest relations"));

    await waitFor(() => expect(screen.getByText(/the quest relations could not be shown/)).toBeInTheDocument());
    fireEvent.click(screen.getByText("NPC"));
    expect(screen.getByText(/ForeverNpcTraces:Load/)).toBeInTheDocument();
  });
});
