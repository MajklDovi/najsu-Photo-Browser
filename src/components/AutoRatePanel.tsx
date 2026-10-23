import { useState } from "react";
import { CATEGORY_LABELS, DEFAULT_WEIGHTS, starsFor, type AutoScore, type Weights } from "../autoScore";
import type { IndexProgress } from "../types";
import "./AutoRatePanel.css";

interface AutoRatePanelProps {
  scores: Map<string, AutoScore>;
  weights: Weights;
  onWeights: (w: Weights) => void;
  analysing: boolean;
  applying: boolean;
  progress: IndexProgress | null;
  message: string | null;
  unratedCount: number;
  detachedCount: number;
  onRestoreStacks: () => void;
  onAnalyse: () => void;
  onApply: (onlyBest: boolean) => void;
}

export function AutoRatePanel({
  scores,
  weights,
  onWeights,
  analysing,
  applying,
  progress,
  message,
  unratedCount,
  detachedCount,
  onRestoreStacks,
  onAnalyse,
  onApply,
}: AutoRatePanelProps) {
  const [showSettings, setShowSettings] = useState(false);
  const [confirm, setConfirm] = useState<"all" | "best" | null>(null);
  const busy = analysing || applying;

  const all = Array.from(scores.values());
  const best = all.filter((s) => s.seriesBest);
  const distribution = [0, 1, 2, 3, 4, 5].map(
    (n) => best.filter((s) => starsFor(s.score) === n).length,
  );

  return (
    <div className="auto-panel">
      <div className="auto-row">
        <button
          onClick={onAnalyse}
          disabled={busy || unratedCount === 0}
          title="Scores every photo that has no rating yet. Runs fully offline."
        >
          ✨ Auto-rate unrated ({unratedCount})
        </button>
        <button className="auto-icon" onClick={() => setShowSettings((v) => !v)} title="Score weights">
          ⚙
        </button>

        {detachedCount > 0 && (
          <button
            onClick={onRestoreStacks}
            title="Put every photo you removed back into its stack"
          >
            ↺ Restore {detachedCount} removed from stacks
          </button>
        )}

        {busy && progress && (
          <span className="auto-status">
            {applying ? "Writing ratings" : "Analysing"}… {progress.done} / {progress.total}
          </span>
        )}
        {!busy && message && <span className="auto-status">{message}</span>}

        {!busy && all.length > 0 && (
          <span className="auto-apply">
            {confirm === null ? (
              <>
                <button onClick={() => setConfirm("best")}>
                  Apply stars to series-best ({best.length})
                </button>
                <button onClick={() => setConfirm("all")}>Apply stars to all ({all.length})</button>
              </>
            ) : (
              <>
                <span className="auto-warning">
                  Write {confirm === "best" ? best.length : all.length} star ratings into your files
                  (EXIF/XMP)? Only photos still unrated are changed.
                </span>
                <button
                  onClick={() => {
                    onApply(confirm === "best");
                    setConfirm(null);
                  }}
                >
                  Yes, write
                </button>
                <button onClick={() => setConfirm(null)}>Cancel</button>
              </>
            )}
          </span>
        )}
      </div>

      {!busy && all.length > 0 && (
        <div className="auto-summary">
          Suggested stars for series-best frames:{" "}
          {distribution.map((count, n) => (
            <span key={n} className="auto-dist">
              {n}★ <b>{count}</b>
            </span>
          ))}
        </div>
      )}

      {showSettings && (
        <div className="auto-settings">
          {(Object.keys(CATEGORY_LABELS) as (keyof typeof CATEGORY_LABELS)[]).map((key) => (
            <label key={key}>
              {CATEGORY_LABELS[key]}
              <input
                type="range"
                min={0}
                max={50}
                value={weights[key]}
                onChange={(e) => onWeights({ ...weights, [key]: Number(e.currentTarget.value) })}
              />
              <span>{weights[key]}</span>
            </label>
          ))}
          <label>
            Series runner-up penalty
            <input
              type="range"
              min={0}
              max={3}
              step={0.5}
              value={weights.seriesPenalty}
              onChange={(e) =>
                onWeights({ ...weights, seriesPenalty: Number(e.currentTarget.value) })
              }
            />
            <span>-{weights.seriesPenalty}★</span>
          </label>
          <p className="auto-hint">
            Changes re-rank instantly (no re-analysis). The (AI) categories only count for photos that
            went through the local AI review.
          </p>
          <button onClick={() => onWeights(DEFAULT_WEIGHTS)}>Reset to defaults</button>
        </div>
      )}
    </div>
  );
}
