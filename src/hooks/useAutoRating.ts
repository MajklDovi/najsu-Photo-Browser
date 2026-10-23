import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { computeAutoScores, DEFAULT_WEIGHTS, starsFor, type Weights } from "../autoScore";
import type { AiResult, AnalysisRecord, IndexProgress, PhotoRecord, SeriesAi } from "../types";

const WEIGHTS_KEY = "najsu:autoWeights";

function loadWeights(): Weights {
  try {
    const raw = localStorage.getItem(WEIGHTS_KEY);
    if (raw) return { ...DEFAULT_WEIGHTS, ...JSON.parse(raw) };
  } catch {
    // fall through to defaults
  }
  return DEFAULT_WEIGHTS;
}

interface ApplyResult {
  written: number;
  skipped: number;
  failed: number;
}

export function useAutoRating(
  folder: string,
  photos: PhotoRecord[],
  reloadCatalog: () => Promise<void>,
) {
  const [records, setRecords] = useState<Map<string, AnalysisRecord>>(new Map());
  const [weights, setWeightsState] = useState<Weights>(loadWeights);
  const [analysing, setAnalysing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  // Photos taken out of their stack. Deliberately in-memory only: nothing is written anywhere.
  const [seriesAi, setSeriesAi] = useState<Map<string, SeriesAi>>(new Map());
  const [detached, setDetached] = useState<Set<string>>(new Set());

  const loadRecords = useCallback(async () => {
    const list = await invoke<AnalysisRecord[]>("load_analysis", { folder });
    setRecords(new Map(list.map((r) => [r.path, r])));
  }, [folder]);

  useEffect(() => {
    setRecords(new Map());
    setMessage(null);
    setDetached(new Set());
    setSeriesAi(new Map());
    loadRecords().catch(() => undefined);
    invoke<SeriesAi[]>("load_series_ai", { folder })
      .then((list) => setSeriesAi(new Map(list.map((v) => [v.series_id, v]))))
      .catch(() => undefined);
  }, [loadRecords, folder]);

  useEffect(() => {
    const subs = [
      listen<AnalysisRecord>("analysis-record", (e) =>
        setRecords((prev) => new Map(prev).set(e.payload.path, e.payload)),
      ),
      listen<{ path: string; ai: AiResult }>("ai-record", (e) =>
        setRecords((prev) => {
          const rec = prev.get(e.payload.path);
          return rec ? new Map(prev).set(rec.path, { ...rec, ai: e.payload.ai }) : prev;
        }),
      ),
      listen<SeriesAi>("ai-series-record", (e) =>
        setSeriesAi((prev) => new Map(prev).set(e.payload.series_id, e.payload)),
      ),
      listen<IndexProgress>("analysis-progress", (e) => setProgress(e.payload)),
      listen<{ analyzed: number; failed: number }>("analysis-complete", (e) => {
        setAnalysing(false);
        setProgress(null);
        const { analyzed, failed } = e.payload;
        setMessage(
          failed > 0
            ? `Analysed ${analyzed} photos (${failed} could not be read, e.g. RAW without a JPEG).`
            : `Analysed ${analyzed} photos.`,
        );
        loadRecords().catch(() => undefined); // pick up the freshly computed series groups
      }),
      listen<string>("analysis-error", (e) => {
        setAnalysing(false);
        setProgress(null);
        setMessage(`Analysis failed: ${e.payload}`);
      }),
      listen<IndexProgress>("apply-progress", (e) => setProgress(e.payload)),
      listen<ApplyResult>("apply-complete", (e) => {
        setApplying(false);
        setProgress(null);
        const { written, skipped, failed } = e.payload;
        setMessage(
          `Wrote ${written} ratings` +
            (skipped ? `, skipped ${skipped} that were rated in the meantime` : "") +
            (failed ? `, ${failed} failed` : "") +
            ".",
        );
        reloadCatalog().catch(() => undefined);
      }),
    ];
    return () => {
      subs.forEach((s) => s.then((f) => f()));
    };
  }, [loadRecords, reloadCatalog]);

  const unrated = useMemo(
    () => new Set(photos.filter((p) => !p.rated).map((p) => p.path)),
    [photos],
  );

  const scores = useMemo(
    () => computeAutoScores(Array.from(records.values()), unrated, weights, detached, seriesAi),
    [records, unrated, weights, detached, seriesAi],
  );

  const setWeights = useCallback((next: Weights) => {
    setWeightsState(next);
    try {
      localStorage.setItem(WEIGHTS_KEY, JSON.stringify(next));
    } catch {
      // per-viewer convenience only
    }
  }, []);

  const detach = useCallback(
    (path: string) => setDetached((prev) => new Set(prev).add(path)),
    [],
  );
  const restoreStacks = useCallback(() => setDetached(new Set()), []);

  const analyse = useCallback(async () => {
    setAnalysing(true);
    setMessage(null);
    await invoke("analyze_folder", { folder });
  }, [folder]);

  // Writes star ratings to unrated photos only. `onlyBest` skips series runner-ups.
  const apply = useCallback(
    async (onlyBest: boolean) => {
      const items = Array.from(scores.values())
        .filter((s) => !onlyBest || s.seriesBest)
        .map((s) => ({ path: s.record.path, rating: starsFor(s.score) }));
      if (items.length === 0) return;
      setApplying(true);
      setMessage(null);
      await invoke("apply_auto_ratings", { folder, items });
    },
    [folder, scores],
  );

  return {
    scores,
    seriesAi,
    weights,
    setWeights,
    analysing,
    applying,
    progress,
    message,
    unratedCount: unrated.size,
    detachedCount: detached.size,
    detach,
    restoreStacks,
    analyse,
    apply,
  };
}
