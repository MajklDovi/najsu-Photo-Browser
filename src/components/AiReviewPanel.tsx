import { useState } from "react";
import type { ReviewScope } from "../aiCandidates";
import type { useAiReview } from "../hooks/useAiReview";
import "./AiReviewPanel.css";

type Ai = ReturnType<typeof useAiReview>;

function formatEta(secs: number): string {
  if (secs < 90) return `${secs}s`;
  if (secs < 5400) return `${Math.round(secs / 60)} min`;
  return `${(secs / 3600).toFixed(1)} h`;
}

function formatSize(bytes: number): string {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

export function AiReviewPanel({ ai }: { ai: Ai }) {
  const { settings, setSettings, status, running, progress, etaSecs, message, candidates } = ai;
  const [showSetup, setShowSetup] = useState(false);
  const reachable = status?.reachable ?? false;
  const noModel = reachable && status!.models.length === 0;
  const canRun = reachable && settings.model !== "" && candidates.length > 0 && !running;

  return (
    <div className="ai-panel">
      <div className="ai-row">
        <button
          onClick={ai.run}
          disabled={!canRun}
          title="Judges composition, subject, people/pose and overall impact with a model running on this computer."
        >
          🧠 AI review ({candidates.length})
        </button>

        <label className="ai-inline">
          Review
          <select
            value={settings.scope}
            onChange={(e) => setSettings({ ...settings, scope: e.currentTarget.value as ReviewScope })}
          >
            <option value="best">best frame of each series</option>
            <option value="top">only the top N photos</option>
            <option value="all">every analysed photo</option>
          </select>
        </label>

        {settings.scope === "top" && (
          <label className="ai-inline" title="One frame per series, ranked by the local score">
            N =
            <input
              type="number"
              min={1}
              max={5000}
              value={settings.topN}
              onChange={(e) =>
                setSettings({ ...settings, topN: Math.max(1, Number(e.currentTarget.value) || 1) })
              }
            />
          </label>
        )}

        <label className="ai-inline" title="Photos the pixel analysis already scores lower than this are skipped">
          skip local score below
          <select
            value={settings.minLocal}
            onChange={(e) => setSettings({ ...settings, minLocal: Number(e.currentTarget.value) })}
          >
            {[0, 1, 1.5, 2, 2.5, 3, 3.5, 4].map((n) => (
              <option key={n} value={n}>
                {n === 0 ? "none" : n}
              </option>
            ))}
          </select>
        </label>

        <span className={`ai-chip ${reachable ? "ai-ok" : "ai-off"}`}>
          {reachable ? `Ollama connected · ${status!.models.length} models` : "Ollama not running"}
        </span>
        <button className="ai-icon" onClick={() => setShowSetup((v) => !v)} title="Local AI setup">
          ⚙
        </button>

        {running && progress && (
          <>
            <span className="ai-status">
              Reviewing {progress.done} / {progress.total}
              {etaSecs !== null && ` · about ${formatEta(etaSecs)} left`}
            </span>
            <button onClick={ai.cancel}>Stop</button>
          </>
        )}
        {!running && message && <span className="ai-status">{message}</span>}
        {!running && !message && ai.analysedCount === 0 && (
          <span className="ai-status">Run Auto-rate first; the AI only looks at analysed photos.</span>
        )}
      </div>

      <div className="ai-row">
        <button
          onClick={ai.runSeries}
          disabled={!(reachable && settings.model !== "" && ai.seriesJobs.length > 0 && !running)}
          title="Shows the model all frames of a burst at once and asks which is the best keeper. Far more reliable than scoring each frame alone."
        >
          ⚖ Compare series ({ai.seriesJobs.length})
        </button>
        <label
          className="ai-inline"
          title="Asks a second time with the frames in reverse order. Small models tend to favour the first or last image; if the two answers disagree the pick falls back to the pixel analysis."
        >
          <input
            type="checkbox"
            checked={settings.doubleCheck}
            onChange={(e) => setSettings({ ...settings, doubleCheck: e.currentTarget.checked })}
          />
          double-check in reverse order (2× slower)
        </label>
        <span className="ai-status">
          Shows every frame of a burst to the model together (at most 6 per series, the best by pixel analysis).
        </span>
      </div>

      {(showSetup || !reachable || noModel) && (
        <div className="ai-setup">
          <p className="ai-privacy">
            🔒 Runs entirely on this computer through Ollama. Photos are resized in memory and sent only to the address
            below; this build refuses any address that isn't <code>localhost</code> and has no internet (https) support
            for the AI at all.
          </p>

          <label>
            Ollama address
            <input
              type="text"
              value={settings.endpoint}
              onChange={(e) => setSettings({ ...settings, endpoint: e.currentTarget.value })}
            />
          </label>

          <label>
            Model
            {status && status.models.length > 0 ? (
              <select
                value={settings.model}
                onChange={(e) => setSettings({ ...settings, model: e.currentTarget.value })}
              >
                {!status.models.some((m) => m.name === settings.model) && <option value="">choose…</option>}
                {status.models.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name} ({formatSize(m.size_bytes)})
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                placeholder="e.g. gemma3:4b"
                value={settings.model}
                onChange={(e) => setSettings({ ...settings, model: e.currentTarget.value })}
              />
            )}
          </label>

          <label>
            Image size sent to the model
            <select
              value={settings.imageEdge}
              onChange={(e) => setSettings({ ...settings, imageEdge: Number(e.currentTarget.value) })}
            >
              <option value={512}>512 px (fastest)</option>
              <option value={768}>768 px (balanced)</option>
              <option value={1024}>1024 px (slowest, most detail)</option>
            </select>
          </label>

          <button onClick={() => ai.check()}>Check connection</button>

          {status && !reachable && (
            <div className="ai-help">
              <b>Ollama isn't reachable.</b> {status.error}
              <ol>
                <li>Install Ollama from ollama.com (about 1 GB) and start it.</li>
                <li>
                  In a terminal run <code>ollama pull gemma3:4b</code> (about 3 GB), or any other model that accepts
                  images.
                </li>
                <li>Come back here and press "Check connection".</li>
              </ol>
              A 3–4B model works on a CPU, but expect tens of seconds per photo, which is why only the best frame of each
              series is reviewed by default.
            </div>
          )}
          {noModel && (
            <div className="ai-help">
              Ollama is running but has no models yet. Run <code>ollama pull gemma3:4b</code> in a terminal, then check
              again.
            </div>
          )}
        </div>
      )}
    </div>
  );
}
