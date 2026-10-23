import { describe, expect, it } from "vitest";
import { pickSeriesJobs } from "./aiCandidates";
import { computeAutoScores, DEFAULT_WEIGHTS } from "./autoScore";
import type { AnalysisRecord, SeriesAi } from "./types";

function rec(name: string, quality: number, series: string | null = "S"): AnalysisRecord {
  return {
    path: `/p/${name}.jpg`,
    sharpness: quality,
    shake: quality,
    exposure: quality,
    noise: quality,
    horizon: null,
    horizon_deg: null,
    series_id: series,
    ai: null,
  };
}

const p = (n: string) => `/p/${n}.jpg`;

function verdict(order: string[], extra: Partial<SeriesAi> = {}): SeriesAi {
  return {
    series_id: "S",
    ranking: order.map(p),
    points: order.map((_, i) => order.length - i),
    reason: "Eyes open, best expression.",
    model: "m",
    passes: 1,
    ...extra,
  };
}

function scoresWith(recs: AnalysisRecord[], v?: SeriesAi, detached = new Set<string>()) {
  const verdicts = new Map(v ? [[v.series_id, v]] : []);
  return computeAutoScores(recs, new Set(recs.map((r) => r.path)), DEFAULT_WEIGHTS, detached, verdicts);
}

describe("AI side-by-side verdict", () => {
  // Pixel analysis ranks a > b > c, with all three technically close.
  const recs = [rec("a", 4.8), rec("b", 4.5), rec("c", 4.2)];

  it("keeps the pixel ranking when there is no verdict", () => {
    const s = scoresWith(recs);
    expect(s.get(p("a"))!.seriesBest).toBe(true);
    expect(s.get(p("a"))!.seriesByAi).toBe(false);
  });

  it("lets the AI's pick win over the pixel ranking when the frames are technically close", () => {
    const s = scoresWith(recs, verdict(["c", "a", "b"]));
    expect(s.get(p("c"))!.seriesBest).toBe(true);
    expect(s.get(p("c"))!.seriesPos).toBe(1);
    expect(s.get(p("a"))!.seriesPos).toBe(2);
    expect(s.get(p("b"))!.seriesPos).toBe(3);
    expect(s.get(p("c"))!.seriesByAi).toBe(true);
    expect(s.get(p("c"))!.seriesReason).toContain("expression");
  });

  it("penalises the runners-up but not the AI's winner", () => {
    const s = scoresWith(recs, verdict(["c", "a", "b"]));
    expect(s.get(p("c"))!.score).toBeCloseTo(s.get(p("c"))!.base);
    expect(s.get(p("a"))!.score).toBeCloseTo(s.get(p("a"))!.base - DEFAULT_WEIGHTS.seriesPenalty);
  });

  it("refuses to crown a frame that is technically far worse, however much the AI likes it", () => {
    const blurry = [rec("a", 4.8), rec("b", 4.5), rec("c", 1.5)]; // c is 3.3 below the best
    const s = scoresWith(blurry, verdict(["c", "a", "b"]));
    expect(s.get(p("c"))!.seriesBest).toBe(false);
    expect(s.get(p("c"))!.seriesVetoed).toBe(true);
    expect(s.get(p("c"))!.seriesPos).toBe(3);
    // Among the technically acceptable frames the AI's order still decides.
    expect(s.get(p("a"))!.seriesBest).toBe(true);
  });

  it("falls back to pixel quality for ties, i.e. when the two passes disagreed", () => {
    const tied = verdict(["b", "c", "a"], { points: [4, 4, 4], passes: 2 });
    const s = scoresWith(recs, tied);
    expect(s.get(p("a"))!.seriesBest).toBe(true);
    // The tie was broken by the pixel analysis, so the AI must not be credited or quoted.
    expect(s.get(p("a"))!.seriesByAi).toBe(false);
    expect(s.get(p("a"))!.seriesReason).toBeNull();
  });

  it("copes with a frame the AI never saw, ranking it after the compared ones", () => {
    const six = [...recs, rec("d", 4.7)];
    const s = scoresWith(six, verdict(["c", "b", "a"])); // d wasn't part of the comparison
    expect(s.get(p("c"))!.seriesBest).toBe(true);
    expect(s.get(p("d"))!.seriesPos).toBe(4);
  });

  it("still works after the user removed a frame from the stack", () => {
    const s = scoresWith(recs, verdict(["c", "a", "b"]), new Set([p("c")]));
    expect(s.get(p("c"))!.seriesSize).toBe(1); // standalone now
    expect(s.get(p("a"))!.seriesBest).toBe(true);
    expect(s.get(p("a"))!.seriesPos).toBe(1);
  });

  it("ignores a verdict that covers fewer than two of the current frames", () => {
    const stale = verdict(["x", "y"]);
    const s = scoresWith(recs, stale);
    expect(s.get(p("a"))!.seriesByAi).toBe(false);
  });
});

describe("pickSeriesJobs", () => {
  const recs = [rec("a", 4.8), rec("b", 4.5), rec("c", 4.2), rec("solo", 4, null), rec("x", 3, "T"), rec("y", 3.5, "T")];
  const scores = scoresWith(recs);

  it("creates one job per multi-frame series, frames in shooting order, strongest series first", () => {
    const jobs = pickSeriesJobs(scores, new Map());
    expect(jobs.map((j) => j.id)).toEqual(["S", "T"]);
    expect(jobs[0].paths).toEqual([p("a"), p("b"), p("c")]);
    expect(jobs[1].paths).toEqual([p("x"), p("y")]);
  });

  it("skips series that already have a verdict covering their frames", () => {
    const done = new Map([["S", verdict(["a", "b", "c"])]]);
    expect(pickSeriesJobs(scores, done).map((j) => j.id)).toEqual(["T"]);
  });

  it("re-queues a series whose verdict no longer covers the frames to compare", () => {
    const stale = new Map([["S", verdict(["a", "b"])]]); // c appeared since
    expect(pickSeriesJobs(scores, stale).map((j) => j.id)).toContain("S");
  });

  it("caps long bursts to the technically best frames", () => {
    const burst = ["a", "b", "c", "d", "e", "f", "g", "h"].map((n, i) => rec(n, 5 - i * 0.2));
    const jobs = pickSeriesJobs(scoresWith(burst), new Map(), 4);
    expect(jobs[0].paths).toEqual([p("a"), p("b"), p("c"), p("d")]);
  });
});
