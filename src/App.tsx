import { useState } from "react";
import { FolderTree } from "./components/FolderTree";
import "./App.css";

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
    <main className="container">
      <h2>Browsing: {confirmedFolder}</h2>
      <p>Indexing and thumbnail grid coming next.</p>
      <button onClick={() => setConfirmedFolder(null)}>Choose a different folder</button>
    </main>
  );
}

export default App;
