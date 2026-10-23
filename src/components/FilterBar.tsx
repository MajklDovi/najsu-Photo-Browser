import "./FilterBar.css";

export type SortKey = "name" | "rating" | "date" | "auto";

export interface FilterState {
  nameFilter: string;
  minRating: number;
  // Minimum "effective" score: your star rating if rated, otherwise the auto score.
  minScore: number;
  seriesBestOnly: boolean;
  stackSeries: boolean;
  // Keep only the N highest-scoring photos (0 = off).
  topN: number;
  sortKey: SortKey;
  sortDesc: boolean;
}

interface FilterBarProps {
  state: FilterState;
  onChange: (state: FilterState) => void;
  totalCount: number;
  filteredCount: number;
  stackCount: number;
}

export function FilterBar({ state, onChange, totalCount, filteredCount, stackCount }: FilterBarProps) {
  return (
    <div className="filter-bar">
      <input
        type="text"
        placeholder="Filter by filename…"
        value={state.nameFilter}
        onChange={(e) => onChange({ ...state, nameFilter: e.currentTarget.value })}
      />

      <label className="filter-label">
        Min rating
        <select
          value={state.minRating}
          onChange={(e) => onChange({ ...state, minRating: Number(e.currentTarget.value) })}
        >
          {[0, 1, 2, 3, 4, 5].map((n) => (
            <option key={n} value={n}>
              {n === 0 ? "Any" : `${n}+ ★`}
            </option>
          ))}
        </select>
      </label>

      <label className="filter-label" title="Your star rating if the photo is rated, otherwise its auto score">
        Score
        <select
          value={state.minScore}
          onChange={(e) => onChange({ ...state, minScore: Number(e.currentTarget.value) })}
        >
          {[0, 1, 2, 2.5, 3, 3.5, 4, 4.5].map((n) => (
            <option key={n} value={n}>
              {n === 0 ? "Any" : `${n}+`}
            </option>
          ))}
        </select>
      </label>

      <label className="filter-label" title="Hide the weaker frames of burst/series shots (unrated photos only)">
        <input
          type="checkbox"
          checked={state.seriesBestOnly}
          onChange={(e) => onChange({ ...state, seriesBestOnly: e.currentTarget.checked })}
        />
        Series best only
      </label>

      <label className="filter-label" title="Group each burst/series into one stack showing its best frame">
        <input
          type="checkbox"
          checked={state.stackSeries}
          onChange={(e) => onChange({ ...state, stackSeries: e.currentTarget.checked })}
        />
        Stack series
      </label>

      <label className="filter-label" title="Keep only the N best photos by score (your stars, or the auto score if unrated). Combine with 'Series best only' to avoid near-duplicates.">
        Best
        <input
          type="number"
          min={0}
          placeholder="all"
          value={state.topN === 0 ? "" : state.topN}
          onChange={(e) => onChange({ ...state, topN: Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)) })}
        />
      </label>

      <label className="filter-label">
        Sort by
        <select
          value={state.sortKey}
          onChange={(e) => onChange({ ...state, sortKey: e.currentTarget.value as SortKey })}
        >
          <option value="name">Name</option>
          <option value="rating">Rating</option>
          <option value="date">Date modified</option>
          <option value="auto">Score (stars / auto)</option>
        </select>
      </label>

      <button onClick={() => onChange({ ...state, sortDesc: !state.sortDesc })}>
        {state.sortDesc ? "↓ Desc" : "↑ Asc"}
      </button>

      <span className="filter-count">
        {filteredCount} / {totalCount}
        {state.stackSeries && stackCount > 0 && ` · ${stackCount} stacks`}
      </span>
    </div>
  );
}
