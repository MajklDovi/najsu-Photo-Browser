import { useMemo, useState } from "react";
import { FolderTree } from "./components/FolderTree";
import { PhotoGrid } from "./components/PhotoGrid";
import { FilterBar, type FilterState } from "./components/FilterBar";
import { PreviewModal } from "./components/PreviewModal";
import { usePhotoCatalog } from "./hooks/usePhotoCatalog";
import "./App.css";

const DEFAULT_FILTER: FilterState = {
  nameFilter: "",
  minRating: 0,
  sortKey: "name",
  sortDesc: false,
};

function BrowseView({ folder, onBack }: { folder: string; onBack: () => void }) {
  const { photos, progress, indexing, error, refresh, setRating } = usePhotoCatalog(folder);
  const [filter, setFilter] = useState<FilterState>(DEFAULT_FILTER);
  const [openIndex, setOpenIndex] = useState<number | null>(null);

  const visiblePhotos = useMemo(() => {
    const needle = filter.nameFilter.trim().toLowerCase();
    const filtered = photos.filter((p) => {
      if (needle && !p.filename.toLowerCase().includes(needle)) return false;
      if (p.rating < filter.minRating) return false;
      return true;
    });

    const sorted = [...filtered].sort((a, b) => {
      switch (filter.sortKey) {
        case "rating":
          return a.rating - b.rating;
        case "date":
          return a.mtime - b.mtime;
        default:
          return a.filename.localeCompare(b.filename, undefined, { sensitivity: "base" });
      }
    });
    if (filter.sortDesc) sorted.reverse();
    return sorted;
  }, [photos, filter]);

  return (
    <div className="browse-view">
      <header className="browse-header">
        <button onClick={onBack}>&larr; Choose a different folder</button>
        <span className="browse-folder-path">{folder}</span>
        <button onClick={refresh} disabled={indexing}>
          Refresh
        </button>
      </header>
      <FilterBar
        state={filter}
        onChange={setFilter}
        totalCount={photos.length}
        filteredCount={visiblePhotos.length}
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
      <PhotoGrid photos={visiblePhotos} onRate={setRating} onOpen={setOpenIndex} />

      {openIndex !== null && (
        <PreviewModal
          photos={visiblePhotos}
          index={openIndex}
          onClose={() => setOpenIndex(null)}
          onNavigate={(next) => {
            if (next >= 0 && next < visiblePhotos.length) setOpenIndex(next);
          }}
          onRate={setRating}
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
    <BrowseView folder={confirmedFolder} onBack={() => setConfirmedFolder(null)} />
  );
}

export default App;
