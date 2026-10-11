import { describe, expect, it } from "vitest";
import { attribute } from "./attribute";
import type { BlockedMoment, BlockedQuest } from "./blocked";

function moment(character: string, keys: string[]): BlockedMoment {
  return {
    character,
    ref: `episode-${character}`,
    t: 1,
    what: "npc:1 list at 1.0s lacked 100",
    transition: false,
    suspects: keys.map((key) => ({ key, weight: 1, detail: key })),
    residualWeight: 0,
  };
}

const params = { unknownWeight: 0.1, emIterations: 30 };

describe("attribute", () => {
  it("gives the blame to the suspect that explains every character, not to its proxies", () => {
    // completed:7 is the real blocker; 8 and 9 are quests players usually do right after it.
    const blocked: BlockedQuest = {
      characters: new Set(["a", "b", "c"]),
      unexplained: new Set(),
      moments: [moment("a", ["completed:7", "completed:8"]), moment("b", ["completed:7", "completed:9"]), moment("c", ["completed:7"])],
    };
    const shares = attribute(blocked, params);

    expect(shares.get("completed:7")!.get("a")!.share).toBeGreaterThan(0.85);
    expect(shares.get("completed:8")!.get("a")!.share).toBeLessThan(0.1);
  });

  it("leaves a lone suspect weak when most blocks of the quest have no plausible suspect", () => {
    // Characters b and c only had unrelated suspects (lumped into the residual): something we
    // cannot see blocks this quest, so a's single related suspect is probably that too.
    const unexplained = (character: string) => ({ ...moment(character, []), residualWeight: 3 });
    const blocked: BlockedQuest = {
      characters: new Set(["a", "b", "c"]),
      unexplained: new Set(),
      moments: [{ ...moment("a", ["inLog:5"]), residualWeight: 3 }, unexplained("b"), unexplained("c")],
    };

    expect(attribute(blocked, params).get("inLog:5")!.get("a")!.share).toBeLessThan(0.3);
  });
});
