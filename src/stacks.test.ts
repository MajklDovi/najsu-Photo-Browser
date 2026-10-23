import { describe, expect, it } from "vitest";
import { computeAutoScores, DEFAULT_WEIGHTS } from "./autoScore";
import { buildGridItems } from "./stacks";
import type { AiResult, AnalysisRecord, PhotoRecord } from "./types";

function photo(name: string, rated = false): PhotoRecord {
  return {
    path: `/p/${name}.jpg`,
    raw_path: null,
    filename: `${name}.jpg`,
    mtime: 0,
    width: null,
    height: null,
    rating: rated ? 3 : 0,
    rated,
    thumbnail: null,
  };
}

// `quality` drives every category so the "better" frame is unambiguous.
function record(name: string, quality: number, series: string | null): AnalysisRecord {
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

const names = ["a", "b", "c", "x"];
const photos = names.map((n) => photo(n));
const records = [
  record("a", 3, "S1"),
  record("b", 4.5, "S1"), // best of the series
  record("c", 2, "S1"),
  record("x", 4, null), // standalone
];
const unrated = new Set(photos.map((p) => p.path));
const scoresOf = (detached = new Set<string>()) =>
  computeAutoScores(records, unrated, DEFAULT_WEIGHTS, detached);

describe("stacks", () => {
  it("collapses a series into one cover: its best frame", () => {
    const items = buildGridItems(photos, scoresOf(), true, new Set());
    expect(items.map((i) => i.photo.filename)).toEqual(["b.jpg", "x.jpg"]);
    expect(items[0].stack).toMatchObject({ id: "S1", size: 3, isCover: true, expanded: false });
    expect(items[1].stack).toBeNull();
  });

  it("expands to show every frame, marking only the best as cover", () => {
    const items = buildGridItems(photos, scoresOf(), true, new Set(["S1"]));
    expect(items.map((i) => i.photo.filename)).toEqual(["a.jpg", "b.jpg", "c.jpg", "x.jpg"]);
    expect(items.filter((i) => i.stack?.isCover).map((i) => i.photo.filename)).toEqual(["b.jpg"]);
    expect(items.every((i) => i.stack === null || i.stack.expanded)).toBe(true);
  });

  it("shows plain photos when stacking is switched off", () => {
    const items = buildGridItems(photos, scoresOf(), false, new Set());
    expect(items).toHaveLength(4);
    expect(items.every((i) => i.stack === null)).toBe(true);
  });

  it("re-picks the cover when the best frame is removed from the stack", () => {
    const detached = new Set(["/p/b.jpg"]);
    const items = buildGridItems(photos, scoresOf(detached), true, new Set());
    // b stands alone now; a (3.0) beats c (2.0) as the new cover of the remaining two.
    expect(items.map((i) => i.photo.filename)).toEqual(["a.jpg", "b.jpg", "x.jpg"]);
    expect(items[0].stack).toMatchObject({ size: 2, isCover: true });
    expect(items[1].stack).toBeNull();
  });

  it("dissolves a stack that is down to one frame", () => {
    const detached = new Set(["/p/a.jpg", "/p/c.jpg"]);
    const items = buildGridItems(photos, scoresOf(detached), true, new Set());
    expect(items.every((i) => i.stack === null)).toBe(true);
    expect(items).toHaveLength(4);
  });

  it("only stacks the frames that survive the active filters", () => {
    const visible = photos.filter((p) => p.filename !== "b.jpg"); // best frame filtered out
    const items = buildGridItems(visible, scoresOf(), true, new Set());
    expect(items.map((i) => i.photo.filename)).toEqual(["a.jpg", "x.jpg"]);
    expect(items[0].stack).toMatchObject({ size: 2, isCover: true });
  });

  it("never stacks rated photos", () => {
    const mixed = [photo("a", true), photo("b"), photo("c")];
    const items = buildGridItems(mixed, scoresOf(), true, new Set());
    expect(items.find((i) => i.photo.filename === "a.jpg")?.stack).toBeNull();
  });
});

describe("series scoring", () => {
  it("penalises runner-ups and leaves detached photos unpenalised", () => {
    const withSeries = scoresOf();
    const b = withSeries.get("/p/b.jpg")!;
    const a = withSeries.get("/p/a.jpg")!;
    expect(b.seriesBest).toBe(true);
    expect(a.score).toBeCloseTo(a.base - DEFAULT_WEIGHTS.seriesPenalty);

    const detached = scoresOf(new Set(["/p/a.jpg"])).get("/p/a.jpg")!;
    expect(detached.seriesSize).toBe(1);
    expect(detached.score).toBeCloseTo(detached.base);
  });
});

describe("AI categories in the score", () => {
  const ai = (over: Partial<AiResult> = {}): AiResult => ({
    model: "m",
    composition: 4,
    subject: 4,
    people: null,
    impact: 4,
    summary: "",
    ...over,
  });
  const scoreOf = (rec: AnalysisRecord) =>
    computeAutoScores([rec], new Set([rec.path]), DEFAULT_WEIGHTS).get(rec.path)!;

  it("ignores AI categories until a photo has been reviewed", () => {
    const base = record("a", 3, null);
    expect(scoreOf(base).score).toBeCloseTo(3);
  });

  it("blends AI scores in with the weights", () => {
    const plain = scoreOf(record("a", 3, null)).score;
    const reviewed = scoreOf({ ...record("a", 3, null), ai: ai({ composition: 5, subject: 5, impact: 5 }) }).score;
    expect(reviewed).toBeGreaterThan(plain);
    const poor = scoreOf({ ...record("a", 3, null), ai: ai({ composition: 1, subject: 1, impact: 1 }) }).score;
    expect(poor).toBeLessThan(plain);
  });

  it("skips the people category when nobody is visible", () => {
    const noPeople = scoreOf({ ...record("a", 4, null), ai: ai({ people: null }) });
    expect(noPeople.score).toBeCloseTo(4); // every category is 4, so the blend stays 4
  });

  it("caps an otherwise great portrait when the people score is bad", () => {
    const rec = { ...record("a", 5, null), ai: ai({ composition: 5, subject: 5, impact: 5, people: 0.5 }) };
    const result = scoreOf(rec);
    expect(result.score).toBeLessThanOrEqual(2.5);
    expect(result.cappedBy).toBe("people / pose");
  });
});

describe("series ranking when only some frames were AI-reviewed", () => {
  const strictAi: AiResult = { model: "m", composition: 3, subject: 3, people: null, impact: 3, summary: "" };
  const lenientAi: AiResult = { model: "m", composition: 5, subject: 5, people: null, impact: 5, summary: "" };
  const rank = (recs: AnalysisRecord[]) =>
    computeAutoScores(recs, new Set(recs.map((r) => r.path)), DEFAULT_WEIGHTS);

  it("keeps the locally-best frame best even though the AI scored it lower than its neighbour's default", () => {
    // a is better on the pixels (4.8 vs 4.4). Only a was reviewed, and the AI was strict.
    const scores = rank([{ ...record("a", 4.8, "S"), ai: strictAi }, record("b", 4.4, "S")]);
    expect(scores.get("/p/a.jpg")!.seriesBest).toBe(true);
    expect(scores.get("/p/b.jpg")!.seriesBest).toBe(false);
    // ...while a's displayed score still reflects the AI.
    expect(scores.get("/p/a.jpg")!.score).toBeLessThan(scores.get("/p/a.jpg")!.localBase);
  });

  it("lets the AI reorder frames once every frame has been reviewed", () => {
    const scores = rank([
      { ...record("a", 4.8, "S"), ai: strictAi },
      { ...record("b", 4.4, "S"), ai: lenientAi },
    ]);
    expect(scores.get("/p/b.jpg")!.seriesBest).toBe(true);
  });

  it("shows the series' best frame as the stack cover", () => {
    const recs = [{ ...record("a", 4.8, "S"), ai: strictAi }, record("b", 4.4, "S")];
    const ps = [photo("a"), photo("b")];
    const items = buildGridItems(ps, rank(recs), true, new Set());
    expect(items.map((i) => i.photo.filename)).toEqual(["a.jpg"]);
  });
});
