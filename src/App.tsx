import { useState } from "react";
import { FolderTree } from "./components/FolderTree";
import { PhotoGrid } from "./components/PhotoGrid";
import { usePhotoCatalog } from "./hooks/usePhotoCatalog";
import "./App.css";

function BrowseView({ folder, onBack }: { folder: string; onBack: () => void }) {
  const { photos, progress, indexing, error, refresh, setRating } = usePhotoCatalog(folder);

  return (
    <div className="browse-view">
      <header className="browse-header">
        <button onClick={onBack}>&larr; Choose a different folder</button>
        <span className="browse-folder-path">{folder}</span>
        <button onClick={refresh} disabled={indexing}>
          Refresh
        </button>
      </header>
      {error && <p className="tree-error">{error}</p>}
      {indexing && (
        <p className="browse-status">
          Indexing… {progress ? `${progress.done} / ${progress.total}` : ""}
        </p>
      )}
      {!indexing && photos.length === 0 && !error && (
        <p className="browse-status">No supported images found in this folder.</p>
      )}
      <PhotoGrid photos={photos} onRate={setRating} />
    </div>
  );
}

function App() {
  const [confirmedFolder, setConfirmedFolder] = useState<string | null>(null);

  if (!confirmedFolder) {
    return (
      <main className="container">
        <FolderTree onConfirm={setConfirmedFolder} />
      </main>
    );
  }

  return (
    <BrowseView folder={confirmedFolder} onBack={() => setConfirmedFolder(null)} />
  );
}

export default App;
