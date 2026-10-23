import { describe, expect, it } from "vitest";
import { pickCandidates } from "./aiCandidates";
import { computeAutoScores, DEFAULT_WEIGHTS } from "./autoScore";
import type { AiResult, AnalysisRecord } from "./types";

function rec(name: string, quality: number, series: string | null, ai: AiResult | null = null): AnalysisRecord {
  return {
    path: `/p/${name}.jpg`,
    sharpness: quality,
    shake: quality,
    exposure: quality,
    noise: quality,
    horizon: null,
    horizon_deg: null,
    series_id: series,
    ai,
  };
}

const reviewed: AiResult = { model: "m", composition: 3, subject: 3, people: null, impact: 3, summary: "" };

// s1: a(4.5) b(3) c(2) - a is the best frame; s2: d(4) e(1); singles f(3.5), g(0.5)
function scoresFor(records: AnalysisRecord[]) {
  return computeAutoScores(records, new Set(records.map((r) => r.path)), DEFAULT_WEIGHTS);
}
const records = [
  rec("a", 4.5, "s1"),
  rec("b", 3, "s1"),
  rec("c", 2, "s1"),
  rec("d", 4, "s2"),
  rec("e", 1, "s2"),
  rec("f", 3.5, null),
  rec("g", 0.5, null),
];
const names = (paths: string[]) => paths.map((p) => p.replace("/p/", "").replace(".jpg", ""));

describe("pickCandidates", () => {
  it("'best' reviews one frame per series plus standalone photos, strongest first", () => {
    const out = pickCandidates(scoresFor(records), { scope: "best", topN: 50, minLocal: 0 });
    expect(names(out)).toEqual(["a", "d", "f", "g"]);
  });

  it("'all' reviews every analysed photo above the minimum", () => {
    const out = pickCandidates(scoresFor(records), { scope: "all", topN: 50, minLocal: 2 });
    expect(names(out)).toEqual(["a", "d", "f", "b", "c"]);
  });

  it("'top' keeps the N best series-best photos", () => {
    const out = pickCandidates(scoresFor(records), { scope: "top", topN: 2, minLocal: 0 });
    expect(names(out)).toEqual(["a", "d"]);
  });

  it("skips photos below the local-score threshold", () => {
    const out = pickCandidates(scoresFor(records), { scope: "best", topN: 50, minLocal: 3 });
    expect(names(out)).toEqual(["a", "d", "f"]);
  });

  it("'top' counts already-reviewed photos towards N, so a second run doesn't creep down the ranking", () => {
    const withReview = [rec("a", 4.5, "s1", reviewed), ...records.slice(1)];
    const out = pickCandidates(scoresFor(withReview), { scope: "top", topN: 2, minLocal: 0 });
    // a is in the top 2 but already done; only d remains. f must NOT be pulled in.
    expect(names(out)).toEqual(["d"]);
  });

  it("returns nothing when everything is reviewed or topN is zero", () => {
    expect(pickCandidates(scoresFor(records), { scope: "top", topN: 0, minLocal: 0 })).toEqual([]);
    const all = records.map((r) => ({ ...r, ai: reviewed }));
    expect(pickCandidates(scoresFor(all), { scope: "all", topN: 50, minLocal: 0 })).toEqual([]);
  });
});
