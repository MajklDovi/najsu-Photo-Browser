import "./FilterBar.css";

export type SortKey = "name" | "rating" | "date";

export interface FilterState {
  nameFilter: string;
  minRating: number;
  sortKey: SortKey;
  sortDesc: boolean;
}

interface FilterBarProps {
  state: FilterState;
  onChange: (state: FilterState) => void;
  totalCount: number;
  filteredCount: number;
}

export function FilterBar({ state, onChange, totalCount, filteredCount }: FilterBarProps) {
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

      <label className="filter-label">
        Sort by
        <select
          value={state.sortKey}
          onChange={(e) => onChange({ ...state, sortKey: e.currentTarget.value as SortKey })}
        >
          <option value="name">Name</option>
          <option value="rating">Rating</option>
          <option value="date">Date modified</option>
        </select>
      </label>

      <button onClick={() => onChange({ ...state, sortDesc: !state.sortDesc })}>
        {state.sortDesc ? "↓ Desc" : "↑ Asc"}
      </button>

      <span className="filter-count">
        {filteredCount} / {totalCount}
      </span>
    </div>
  );
}
