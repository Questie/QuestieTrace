import { describe, expect, it } from "vitest";
import { fitLogistic, predict } from "./logistic";

/** mulberry32: a seeded generator, so the sample (and the test) never changes. */
function random(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

describe("fitLogistic", () => {
  it("recovers the weights that generated the labels", () => {
    const next = random(7);
    const truth = { intercept: -0.5, weights: [2, -1] };
    const rows = Array.from({ length: 5000 }, () => [next() < 0.5 ? 1 : 0, next() * 4 - 2]);
    const labels = rows.map((x) => (next() < predict(truth, x) ? 1 : 0));

    const model = fitLogistic(rows, labels, { ridge: 1e-6 });

    expect(model.intercept).toBeCloseTo(truth.intercept, 0);
    expect(model.weights[0]).toBeCloseTo(truth.weights[0], 0);
    expect(model.weights[1]).toBeCloseTo(truth.weights[1], 0);
  });

  it("shrinks toward the prior, not toward zero", () => {
    const prior = { intercept: 0.3, weights: [0, 3] };
    // The second column is never set, so the data says nothing about it and the prior stands.
    const rows = Array.from({ length: 200 }, (_, i) => [i % 2, 0]);
    const labels = rows.map((_, i) => (i % 4 === 0 ? 1 : 0));

    const model = fitLogistic(rows, labels, { ridge: 1, prior });

    expect(model.weights[1]).toBeCloseTo(3, 6);
    expect(fitLogistic([], [], { ridge: 1, prior })).toEqual(prior);
  });
});
