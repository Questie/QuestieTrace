import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ExtractView } from "./ExtractView";

function mockFetchOnce(body: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() => Promise.resolve({ json: () => Promise.resolve(body) } as Response)),
  );
}

const successResult = {
  npcFixes:
    'function ForeverTraceNpcFixes:Load()\n    local npcKeys = QuestieDB.npcKeys\n\n    return {\n        [823] = {\n            [npcKeys.name] = "Deputy Willem",\n        },\n    }\nend',
  sessionCount: 3,
  fileCount: 2,
  skippedFiles: [],
};

describe("ExtractView", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should show a Generate button and no output before anything is triggered", () => {
    render(<ExtractView />);

    expect(screen.getByText("Generate")).toBeInTheDocument();
    expect(screen.queryByText(/ForeverTraceNpcFixes:Load/)).not.toBeInTheDocument();
  });

  it("should show a loading state while the extraction request is in flight", () => {
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    expect(screen.getByText("Generating...")).toBeInTheDocument();
    expect(screen.getByText("Generating...")).toBeDisabled();
  });

  it("should render the combined ForeverTraceNpcFixes Lua plus a session/file summary on success", async () => {
    mockFetchOnce(successResult);

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/ForeverTraceNpcFixes:Load/)).toBeInTheDocument());
    expect(screen.getByText("3 session(s) from 2 trace file(s)")).toBeInTheDocument();
    expect(screen.getByText("Deputy Willem", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Copy to clipboard")).toBeInTheDocument();
    expect(screen.getByText("Download ForeverTraceNpcFixes.lua")).toBeInTheDocument();
  });

  it("should list skipped files as a warning when some trace files fail to load", async () => {
    mockFetchOnce({
      ...successResult,
      skippedFiles: [{ name: "bad.lua", error: "unexpected token" }],
    });

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/Skipped 1 file/)).toBeInTheDocument());
    expect(screen.getByText(/bad\.lua: unexpected token/)).toBeInTheDocument();
  });

  it("should show an error message and allow retrying when the request fails", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network down"))));

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/Error: network down/)).toBeInTheDocument());

    mockFetchOnce(successResult);
    fireEvent.click(screen.getByText("Generate"));

    await waitFor(() => expect(screen.getByText(/ForeverTraceNpcFixes:Load/)).toBeInTheDocument());
  });

  it("should trigger a download named ForeverTraceNpcFixes.lua when clicking the download button", async () => {
    mockFetchOnce(successResult);
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView />);
    fireEvent.click(screen.getByText("Generate"));
    await waitFor(() => expect(screen.getByText(/ForeverTraceNpcFixes:Load/)).toBeInTheDocument());

    // Re-stub fetch aside, URL.createObjectURL isn't provided by jsdom.
    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const createElementSpy = vi.spyOn(document, "createElement");

    fireEvent.click(screen.getByText("Download ForeverTraceNpcFixes.lua"));

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("ForeverTraceNpcFixes.lua");
    expect(clickSpy).toHaveBeenCalled();
  });
});
