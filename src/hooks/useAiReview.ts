import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AutoScore } from "../autoScore";
import type { SeriesAi } from "../types";
import { pickCandidates, pickSeriesJobs, type ReviewScope } from "../aiCandidates";

export interface OllamaModel {
  name: string;
  size_bytes: number;
}

export interface OllamaStatus {
  reachable: boolean;
  models: OllamaModel[];
  error: string | null;
}

export interface AiSettings {
  endpoint: string;
  model: string;
  imageEdge: number;
  scope: ReviewScope;
  // "top" scope: how many of the best photos to review.
  topN: number;
  // Skip photos the pixel analysis already scores below this.
  minLocal: number;
  // Series comparison: also ask with the frames reversed to cancel the model's positional bias.
  doubleCheck: boolean;
}

const SETTINGS_KEY = "najsu:aiSettings";
const DEFAULT_SETTINGS: AiSettings = {
  endpoint: "http://localhost:11434",
  model: "",
  imageEdge: 768,
  scope: "best",
  topN: 50,
  minLocal: 2,
  doubleCheck: false,
};

function loadSettings(): AiSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
  } catch {
    // fall through to defaults
  }
  return DEFAULT_SETTINGS;
}

export function useAiReview(
  folder: string,
  scores: Map<string, AutoScore>,
  seriesAi: Map<string, SeriesAi>,
) {
  const [settings, setSettingsState] = useState<AiSettings>(loadSettings);
  const [status, setStatus] = useState<OllamaStatus | null>(null);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [etaSecs, setEtaSecs] = useState<number | null>(null);
  const timing = useRef({ sum: 0, count: 0 });

  const setSettings = useCallback((next: AiSettings) => {
    setSettingsState(next);
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(next));
    } catch {
      // per-viewer convenience only
    }
  }, []);

  const check = useCallback(async () => {
    const result = await invoke<OllamaStatus>("ai_check", { endpoint: settings.endpoint });
    setStatus(result);
    // Nothing chosen yet: pick the first installed model so the common case needs no typing.
    if (result.reachable && !settings.model && result.models.length > 0) {
      setSettings({ ...settings, model: result.models[0].name });
    }
    return result;
  }, [settings, setSettings]);

  useEffect(() => {
    check().catch(() => undefined);
    // Re-check when the address changes; the model list is cheap to fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.endpoint]);

  useEffect(() => {
    const subs = [
      listen<{ done: number; total: number; last_secs: number }>("ai-progress", (e) => {
        const { done, total, last_secs } = e.payload;
        setProgress({ done, total });
        if (done > 0 && last_secs > 0) {
          timing.current.sum += last_secs;
          timing.current.count += 1;
          const avg = timing.current.sum / timing.current.count;
          setEtaSecs(Math.round(avg * (total - done)));
        }
      }),
      listen<{ reviewed: number; failed: number; cancelled: boolean }>("ai-complete", (e) => {
        setRunning(false);
        setProgress(null);
        setEtaSecs(null);
        const { reviewed, failed, cancelled } = e.payload;
        setMessage(
          `${cancelled ? "Stopped. " : ""}Reviewed ${reviewed} photos` +
            (failed ? `, ${failed} failed.` : "."),
        );
      }),
      listen<{ ranked: number; failed: number; cancelled: boolean }>("ai-series-complete", (e) => {
        setRunning(false);
        setProgress(null);
        setEtaSecs(null);
        const { ranked, failed, cancelled } = e.payload;
        setMessage(
          `${cancelled ? "Stopped. " : ""}Compared ${ranked} series` + (failed ? `, ${failed} failed.` : "."),
        );
      }),
      listen<string>("ai-error", (e) => setMessage(`AI review stopped: ${e.payload}`)),
    ];
    return () => {
      subs.forEach((s) => s.then((f) => f()));
    };
  }, []);

  // Which photos a run would cover, given the chosen mode.
  const candidates = useMemo(
    () =>
      pickCandidates(scores, {
        scope: settings.scope,
        topN: settings.topN,
        minLocal: settings.minLocal,
      }),
    [scores, settings.scope, settings.topN, settings.minLocal],
  );

  const seriesJobs = useMemo(() => pickSeriesJobs(scores, seriesAi), [scores, seriesAi]);

  const runSeries = useCallback(async () => {
    if (seriesJobs.length === 0) return;
    timing.current = { sum: 0, count: 0 };
    setRunning(true);
    setMessage(null);
    setEtaSecs(null);
    try {
      await invoke("ai_rank_series", {
        folder,
        jobs: seriesJobs,
        endpoint: settings.endpoint,
        model: settings.model,
        imageEdge: settings.imageEdge,
        doubleCheck: settings.doubleCheck,
      });
    } catch (e) {
      setRunning(false);
      setMessage(String(e));
    }
  }, [seriesJobs, folder, settings]);

  const run = useCallback(async () => {
    if (candidates.length === 0) return;
    timing.current = { sum: 0, count: 0 };
    setRunning(true);
    setMessage(null);
    setEtaSecs(null);
    try {
      await invoke("ai_review", {
        folder,
        paths: candidates,
        endpoint: settings.endpoint,
        model: settings.model,
        imageEdge: settings.imageEdge,
      });
    } catch (e) {
      setRunning(false);
      setMessage(String(e));
    }
  }, [candidates, folder, settings]);

  const cancel = useCallback(() => invoke("ai_cancel"), []);

  return {
    settings,
    setSettings,
    status,
    check,
    running,
    progress,
    etaSecs,
    message,
    candidates,
    seriesJobs,
    runSeries,
    analysedCount: scores.size,
    run,
    cancel,
  };
}
