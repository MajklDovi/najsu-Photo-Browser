import { useEffect, useMemo, useState } from "react";
import { FolderTree } from "./components/FolderTree";
import { PhotoGrid } from "./components/PhotoGrid";
import { FilterBar, type FilterState } from "./components/FilterBar";
import { PreviewModal } from "./components/PreviewModal";
import { SubfolderBar } from "./components/SubfolderBar";
import { AutoRatePanel } from "./components/AutoRatePanel";
import { useAutoRating } from "./hooks/useAutoRating";
import { buildGridItems } from "./stacks";
import { AiReviewPanel } from "./components/AiReviewPanel";
import { useAiReview } from "./hooks/useAiReview";
import { usePhotoCatalog } from "./hooks/usePhotoCatalog";
import type { DragMode } from "./types";
import "./App.css";

const DEFAULT_FILTER: FilterState = {
  nameFilter: "",
  minRating: 0,
  minScore: 0,
  seriesBestOnly: false,
  stackSeries: true,
  topN: 0,
  sortKey: "name",
  sortDesc: false,
};

const DRAG_MODE_KEY = "najsu:dragMode";

function useDragMode() {
  const [dragMode, setDragMode] = useState<DragMode>(
    () => (localStorage.getItem(DRAG_MODE_KEY) as DragMode | null) ?? "both",
  );
  function update(mode: DragMode) {
    localStorage.setItem(DRAG_MODE_KEY, mode);
    setDragMode(mode);
  }
  return [dragMode, update] as const;
}

function BrowseView({ root, onBack }: { root: string; onBack: () => void }) {
  const [currentFolder, setCurrentFolder] = useState(root);
  const [recursive, setRecursive] = useState(false);
  const { photos, progress, indexing, error, refresh, reload, setRating } = usePhotoCatalog(
    currentFolder,
    recursive,
  );
  const [filter, setFilter] = useState<FilterState>(DEFAULT_FILTER);
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const [dragMode, setDragMode] = useDragMode();
  const auto = useAutoRating(currentFolder, photos, reload);
  const aiReview = useAiReview(currentFolder, auto.scores, auto.seriesAi);
  const [expandedStacks, setExpandedStacks] = useState<Set<string>>(new Set());

  useEffect(() => {
    setCurrentFolder(root);
  }, [root]);

  const visiblePhotos = useMemo(() => {
    const needle = filter.nameFilter.trim().toLowerCase();
    // Your star rating if the photo has one, otherwise its auto score (-1 = not scored yet).
    const effective = (p: (typeof photos)[number]) =>
      p.rated ? p.rating : (auto.scores.get(p.path)?.score ?? -1);

    const filtered = photos.filter((p) => {
      if (needle && !p.filename.toLowerCase().includes(needle)) return false;
      if (p.rating < filter.minRating) return false;
      if (filter.minScore > 0 && effective(p) < filter.minScore) return false;
      if (filter.seriesBestOnly) {
        const a = auto.scores.get(p.path);
        if (!p.rated && a && !a.seriesBest) return false;
      }
      return true;
    });

    // "Best N": keep the N highest effective scores, then fall through to the chosen sort.
    const kept =
      filter.topN > 0
        ? [...filtered].sort((a, b) => effective(b) - effective(a)).slice(0, filter.topN)
        : filtered;

    const sorted = [...kept].sort((a, b) => {
      switch (filter.sortKey) {
        case "rating":
          return a.rating - b.rating;
        case "date":
          return a.mtime - b.mtime;
        case "auto":
          return effective(a) - effective(b);
        default:
          return a.filename.localeCompare(b.filename, undefined, { sensitivity: "base" });
      }
    });
    if (filter.sortDesc) sorted.reverse();
    return sorted;
  }, [photos, filter, auto.scores]);

  const items = useMemo(
    () => buildGridItems(visiblePhotos, auto.scores, filter.stackSeries, expandedStacks),
    [visiblePhotos, auto.scores, filter.stackSeries, expandedStacks],
  );
  // The preview walks the same sequence the grid shows (expanded stacks include their frames).
  const previewPhotos = useMemo(() => items.map((i) => i.photo), [items]);

  function toggleStack(id: string) {
    setExpandedStacks((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  return (
    <div className="browse-view">
      <header className="browse-header">
        <button onClick={onBack}>&larr; Choose a different folder</button>
        <label className="include-subfolders">
          <input
            type="checkbox"
            checked={recursive}
            onChange={(e) => setRecursive(e.currentTarget.checked)}
          />
          Include subfolders
        </label>
        <label className="drag-mode">
          Drag &amp; drop
          <select
            value={dragMode}
            onChange={(e) => setDragMode(e.currentTarget.value as DragMode)}
          >
            <option value="jpg">JPG only</option>
            <option value="raw">RAW only</option>
            <option value="both">Both</option>
          </select>
        </label>
        <button onClick={refresh} disabled={indexing}>
          Refresh
        </button>
      </header>
      <SubfolderBar root={root} currentFolder={currentFolder} onNavigate={setCurrentFolder} />
      <AutoRatePanel
        scores={auto.scores}
        weights={auto.weights}
        onWeights={auto.setWeights}
        analysing={auto.analysing}
        applying={auto.applying}
        progress={auto.progress}
        message={auto.message}
        unratedCount={auto.unratedCount}
        detachedCount={auto.detachedCount}
        onRestoreStacks={auto.restoreStacks}
        onAnalyse={auto.analyse}
        onApply={auto.apply}
      />
      <AiReviewPanel ai={aiReview} />
      <FilterBar
        state={filter}
        onChange={setFilter}
        totalCount={photos.length}
        filteredCount={visiblePhotos.length}
        stackCount={items.filter((i) => i.stack?.isCover).length}
      />
      {error && <p className="tree-error">{error}</p>}
      {indexing && (
        <p className="browse-status">
          Indexing… {progress ? `${progress.done} / ${progress.total}` : ""}
        </p>
      )}
      {!indexing && photos.length === 0 && !error && (
        <p className="browse-status">No supported images found in this folder.</p>
      )}
      <PhotoGrid
        items={items}
        onRate={setRating}
        onOpen={setOpenIndex}
        dragMode={dragMode}
        scores={auto.scores}
        onToggleStack={toggleStack}
        onDetach={auto.detach}
      />

      {openIndex !== null && (
        <PreviewModal
          photos={previewPhotos}
          index={openIndex}
          onClose={() => setOpenIndex(null)}
          onNavigate={(next) => {
            if (next >= 0 && next < previewPhotos.length) setOpenIndex(next);
          }}
          onRate={setRating}
          dragMode={dragMode}
          scores={auto.scores}
          onDetach={auto.detach}
        />
      )}
    </div>
  );
}

const LAST_FOLDER_KEY = "najsu:lastFolder";

function App() {
  const [confirmedFolder, setConfirmedFolder] = useState<string | null>(null);
  const [lastFolder, setLastFolder] = useState<string | null>(() =>
    localStorage.getItem(LAST_FOLDER_KEY),
  );

  function confirmFolder(path: string) {
    localStorage.setItem(LAST_FOLDER_KEY, path);
    setLastFolder(path);
    setConfirmedFolder(path);
  }

  if (!confirmedFolder) {
    return (
      <main className="container">
        <FolderTree onConfirm={confirmFolder} lastFolder={lastFolder} />
      </main>
    );
  }

  return (
    <BrowseView root={confirmedFolder} onBack={() => setConfirmedFolder(null)} />
  );
}

export default App;
