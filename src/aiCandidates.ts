import type { AutoScore } from "./autoScore";
import type { SeriesAi } from "./types";

// Which photos the (slow) local AI review should look at.
//   best - the best frame of each series
//   top  - the N highest-scoring photos, one frame per series
//   all  - every analysed photo
export type ReviewScope = "best" | "all" | "top";

export interface CandidateOptions {
  scope: ReviewScope;
  // Only used by the "top" scope.
  topN: number;
  // Photos the pixel analysis already scores below this are not worth model time.
  minLocal: number;
}

// Returns the paths still to review, most promising first, so that stopping a long run
// early still leaves the strongest candidates reviewed.
export function pickCandidates(scores: Map<string, AutoScore>, opts: CandidateOptions): string[] {
  let pool = Array.from(scores.values()).filter((s) => s.base >= opts.minLocal);

  if (opts.scope !== "all") pool = pool.filter((s) => s.seriesBest);
  pool.sort((a, b) => b.base - a.base);

  // "Top N" is decided over everything eligible, reviewed or not, so running it twice
  // doesn't creep down the ranking and spend time on photos outside the top N.
  if (opts.scope === "top") pool = pool.slice(0, Math.max(0, Math.floor(opts.topN)));

  return pool.filter((s) => !s.record.ai).map((s) => s.record.path);
}

export interface SeriesJob {
  id: string;
  // The frames to compare, in shooting (file name) order.
  paths: string[];
}

// Frames compared in one request. Every extra image costs a CPU-only model minutes, and the
// frames that matter are the plausible winners, so a long burst is cut to its best by the
// pixel analysis; the rest simply rank below.
export const MAX_FRAMES_PER_SERIES = 6;

// Series that still need a side-by-side comparison: two or more unrated frames and no
// verdict covering the frames that would be compared.
export function pickSeriesJobs(
  scores: Map<string, AutoScore>,
  verdicts: Map<string, SeriesAi>,
  maxFrames = MAX_FRAMES_PER_SERIES,
): SeriesJob[] {
  const groups = new Map<string, AutoScore[]>();
  for (const s of scores.values()) {
    if (!s.seriesId) continue;
    groups.set(s.seriesId, [...(groups.get(s.seriesId) ?? []), s]);
  }

  const jobs: { job: SeriesJob; strength: number }[] = [];
  for (const [id, members] of groups) {
    if (members.length < 2) continue;
    const chosen = [...members].sort((a, b) => b.localBase - a.localBase).slice(0, maxFrames);
    const paths = chosen.map((m) => m.record.path).sort();

    const done = verdicts.get(id);
    if (done && paths.every((p) => done.ranking.includes(p))) continue;
    jobs.push({ job: { id, paths }, strength: Math.max(...members.map((m) => m.localBase)) });
  }
  // Most promising series first, so stopping early still covers the ones that matter.
  return jobs.sort((a, b) => b.strength - a.strength).map((j) => j.job);
}
