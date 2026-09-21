import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ExtractView } from "./ExtractView";

const sessionSummary = { name: "s1", duration: 10, startedAt: 0, eventCount: 0, functionCount: 1 };
const fullSession = {
  schemaVersion: 9,
  name: "s1",
  startedAt: 0,
  startedAtPrecise: 0,
  stoppedAt: 10,
  stoppedAtPrecise: 10,
  duration: 10,
  durationPrecise: 10,
  events: [],
  functions: {
    UnitGUID: { target: [{ t: 0, tp: 0, v: "Creature-0-5208-0-7-823-000031" }] },
    UnitName: { target: [{ t: 0, tp: 0, v: { 1: "Deputy Willem", n: 2 } }] },
  },
  functionsDelta: {},
};

const PENDING = Symbol("pending");

function mockFetchSequence(responses: unknown[]) {
  let call = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(() => {
      const body = responses[call++];
      if (body === PENDING) {
        return new Promise(() => {}); // never resolves
      }
      return Promise.resolve({ json: () => Promise.resolve(body) } as Response);
    }),
  );
}

describe("ExtractView", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should render the generated npcDB Lua for all sessions in the selected file", async () => {
    mockFetchSequence([[sessionSummary], fullSession]);

    render(<ExtractView fileName="test.lua" />);

    await waitFor(() => expect(screen.getByText(/local npcData = \{/)).toBeInTheDocument());
    expect(screen.getByText(/Deputy Willem/)).toBeInTheDocument();
  });

  it("should show a loading state before sessions have arrived", () => {
    mockFetchSequence([PENDING]);

    render(<ExtractView fileName="test.lua" />);

    expect(screen.getByText(/Loading/i)).toBeInTheDocument();
  });

  it("should show an error message when the sessions request fails", async () => {
    mockFetchSequence([{ error: "boom" }]);

    render(<ExtractView fileName="test.lua" />);

    await waitFor(() => expect(screen.getByText(/Error: boom/)).toBeInTheDocument());
  });

  it("should trigger a download named npcDB.lua when clicking the download button", async () => {
    mockFetchSequence([[sessionSummary], fullSession]);
    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:mock"), revokeObjectURL: vi.fn() });
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<ExtractView fileName="test.lua" />);
    await waitFor(() => expect(screen.getByText(/local npcData = \{/)).toBeInTheDocument());

    const createElementSpy = vi.spyOn(document, "createElement");
    screen.getByText("Download npcDB.lua").click();

    const anchor = createElementSpy.mock.results.find((r) => r.value instanceof HTMLAnchorElement)?.value as
      | HTMLAnchorElement
      | undefined;
    expect(anchor?.download).toBe("npcDB.lua");
    expect(clickSpy).toHaveBeenCalled();
  });
});
