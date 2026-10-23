import type { AnalysisRecord, SeriesAi } from "./types";

// Category weights are relative (they're renormalised over whichever categories exist for a
// photo), so e.g. a photo with no measurable horizon simply ignores that weight.
export interface Weights {
  sharpness: number;
  shake: number;
  exposure: number;
  noise: number;
  horizon: number;
  // These four only exist for photos that went through the local AI review.
  composition: number;
  subject: number;
  people: number;
  impact: number;
  // Stars subtracted from every non-best frame of a series.
  seriesPenalty: number;
}

export const DEFAULT_WEIGHTS: Weights = {
  sharpness: 35,
  shake: 15,
  exposure: 20,
  noise: 10,
  horizon: 5,
  composition: 30,
  subject: 10,
  people: 15,
  impact: 20,
  seriesPenalty: 1,
};

export const CATEGORY_LABELS: Record<keyof Omit<Weights, "seriesPenalty">, string> = {
  sharpness: "Sharpness",
  shake: "Camera shake",
  exposure: "Exposure",
  noise: "Noise",
  horizon: "Horizon level",
  composition: "Composition (AI)",
  subject: "Subject (AI)",
  people: "People / pose (AI)",
  impact: "Overall impact (AI)",
};

export interface AutoScore {
  // Final 0-5 score after caps and the series adjustment.
  score: number;
  // Score before the series adjustment.
  base: number;
  // Score from the pixel analysis alone: the only like-for-like way to rank frames of a series
  // when the AI has reviewed some of them but not others.
  localBase: number;
  seriesId: string | null;
  seriesPos: number;
  seriesSize: number;
  seriesBest: boolean;
  // True when the local AI compared the frames side by side and its verdict set the order.
  seriesByAi: boolean;
  seriesReason: string | null;
  // This frame was the AI's favourite but is technically too weak to be the series' best.
  seriesVetoed: boolean;
  // Why the score was limited, if a hard failure capped it.
  cappedBy: string | null;
  record: AnalysisRecord;
}

// A plain average hides disasters (a perfectly exposed but badly blurred frame would still
// score well), so a failing category puts a ceiling on the whole score.
function baseScore(
  r: AnalysisRecord,
  w: Weights,
  useAi = true,
): { score: number; cappedBy: string | null } {
  const aiResult = useAi ? r.ai : null;
  const parts: [number, number][] = [
    [r.sharpness, w.sharpness],
    [r.shake, w.shake],
    [r.exposure, w.exposure],
    [r.noise, w.noise],
  ];
  if (r.horizon !== null) parts.push([r.horizon, w.horizon]);
  if (aiResult) {
    parts.push([aiResult.composition, w.composition]);
    parts.push([aiResult.subject, w.subject]);
    parts.push([aiResult.impact, w.impact]);
    if (aiResult.people !== null) parts.push([aiResult.people, w.people]);
  }

  const totalWeight = parts.reduce((sum, [, weight]) => sum + weight, 0);
  let score =
    totalWeight > 0
      ? parts.reduce((sum, [value, weight]) => sum + value * weight, 0) / totalWeight
      : 0;

  let cappedBy: string | null = null;
  const caps: [boolean, number, string][] = [
    [r.sharpness < 1, 1.5, "blurred"],
    [r.shake < 1, 2, "camera shake"],
    [r.exposure < 1, 2.5, "exposure"],
    // Closed eyes / a clearly bad pose ruin a portrait however good the rest is.
    [aiResult?.people != null && aiResult.people < 1.5, 2.5, "people / pose"],
  ];
  for (const [failed, cap, label] of caps) {
    if (failed && score > cap) {
      score = cap;
      cappedBy = label;
    }
  }
  return { score, cappedBy };
}

// Scores every photo that is still unrated, then picks the best frame of each series.
export function computeAutoScores(
  records: AnalysisRecord[],
  unrated: Set<string>,
  weights: Weights,
  // Photos the user removed from their stack: treated as standalone, so they get no series
  // penalty and the stack's best frame is re-picked from whoever is left.
  detached: Set<string> = new Set(),
  // The AI's side-by-side verdicts, keyed by series id.
  seriesAi: Map<string, SeriesAi> = new Map(),
): Map<string, AutoScore> {
  const out = new Map<string, AutoScore>();
  const groups = new Map<string, AutoScore[]>();

  for (const record of records) {
    if (!unrated.has(record.path)) continue;
    const { score, cappedBy } = baseScore(record, weights);
    const seriesId = detached.has(record.path) ? null : record.series_id;
    const entry: AutoScore = {
      score,
      base: score,
      localBase: baseScore(record, weights, false).score,
      seriesId,
      seriesPos: 1,
      seriesSize: 1,
      seriesBest: true,
      seriesByAi: false,
      seriesReason: null,
      seriesVetoed: false,
      cappedBy,
      record,
    };
    out.set(record.path, entry);
    if (seriesId) {
      const list = groups.get(seriesId) ?? [];
      list.push(entry);
      groups.set(seriesId, list);
    }
  }

  for (const [seriesId, members] of groups) {
    if (members.length < 2) continue;
    const ordered = orderSeries(members, seriesAi.get(seriesId));
    ordered.forEach((m, i) => {
      m.seriesPos = i + 1;
      m.seriesSize = ordered.length;
      m.seriesBest = i === 0;
      if (i > 0) m.score = Math.max(0, m.base - weights.seriesPenalty);
    });
  }
  return out;
}

// A frame this much worse than the series' technical best can't be crowned by the AI, which
// is told not to judge sharpness and so can happily prefer a blurred frame with a nicer smile.
export const TECHNICAL_VETO = 1.5;

// Orders the frames of one series, best first.
function orderSeries(members: AutoScore[], verdict: SeriesAi | undefined): AutoScore[] {
  const byLocal = (a: AutoScore, b: AutoScore) =>
    b.localBase - a.localBase || b.record.sharpness - a.record.sharpness;

  const points = new Map<string, number>();
  verdict?.ranking.forEach((path, i) => points.set(path, verdict.points[i] ?? 0));
  const usable = verdict && members.filter((m) => points.has(m.record.path)).length >= 2;

  if (!usable) {
    // Frames are only comparable on the same footing. If the AI reviewed just some of them,
    // its (typically stricter) scores would unfairly sink exactly those frames, so rank on the
    // pixel analysis alone; once every frame has an AI review, rank on the full blend.
    const allReviewed = members.every((m) => m.record.ai);
    const rank = (m: AutoScore) => (allReviewed ? m.base : m.localBase);
    return [...members].sort((a, b) => rank(b) - rank(a) || b.record.sharpness - a.record.sharpness);
  }

  const pts = (m: AutoScore) => points.get(m.record.path) ?? -1;
  const byAi = (a: AutoScore, b: AutoScore) => pts(b) - pts(a) || byLocal(a, b);

  const maxLocal = Math.max(...members.map((m) => m.localBase));
  const eligible = members.filter((m) => m.localBase >= maxLocal - TECHNICAL_VETO).sort(byAi);
  const rest = members.filter((m) => m.localBase < maxLocal - TECHNICAL_VETO).sort(byLocal);

  // Note which frame the AI preferred if the technical veto overruled it.
  const aiFavourite = [...members].sort(byAi)[0];
  const vetoed = rest.includes(aiFavourite) ? aiFavourite : null;

  const ordered = [...eligible, ...rest];
  // When the first two frames are level on points (the two passes disagreed) the pixel analysis
  // broke the tie, so don't present the pick, or the model's comment, as the AI's decision.
  const clearWinner = pts(ordered[0]) > pts(ordered[1]);
  for (const m of members) {
    m.seriesByAi = clearWinner;
    m.seriesReason = clearWinner ? verdict!.reason || null : null;
    m.seriesVetoed = m === vetoed;
  }
  return ordered;
}

export function starsFor(score: number): number {
  return Math.max(0, Math.min(5, Math.round(score)));
}
