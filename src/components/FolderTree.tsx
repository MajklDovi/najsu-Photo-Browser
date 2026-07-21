import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DirEntry, DriveEntry } from "../types";
import "./FolderTree.css";

interface TreeNodeProps {
  name: string;
  path: string;
  selectedPath: string | null;
  onSelect: (path: string) => void;
}

function TreeNode({ name, path, selectedPath, onSelect }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<DirEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isSelected = selectedPath === path;

  async function toggle() {
    onSelect(path);
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (children === null) {
      setLoading(true);
      setError(null);
      try {
        const entries = await invoke<DirEntry[]>("list_dir", { path });
        setChildren(entries.filter((e) => e.is_dir));
      } catch (e) {
        setError(String(e));
        setChildren([]);
      } finally {
        setLoading(false);
      }
    }
  }

  return (
    <li>
      <div
        className={`tree-row${isSelected ? " tree-row-selected" : ""}`}
        onClick={toggle}
      >
        <span className={`tree-caret${expanded ? " tree-caret-open" : ""}`}>
          ▶
        </span>
        <span className="tree-label">{name}</span>
      </div>
      {expanded && (
        <ul className="tree-children">
          {loading && <li className="tree-status">Loading…</li>}
          {error && <li className="tree-status tree-error">{error}</li>}
          {children && children.length === 0 && !loading && (
            <li className="tree-status">(no subfolders)</li>
          )}
          {children?.map((child) => (
            <TreeNode
              key={child.path}
              name={child.name}
              path={child.path}
              selectedPath={selectedPath}
              onSelect={onSelect}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

interface FolderTreeProps {
  onConfirm: (path: string) => void;
}

export function FolderTree({ onConfirm }: FolderTreeProps) {
  const [drives, setDrives] = useState<DriveEntry[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    invoke<DriveEntry[]>("list_drives")
      .then(setDrives)
      .catch((e) => setLoadError(String(e)));
  }, []);

  return (
    <div className="folder-tree-container">
      <h2>Choose a starting folder</h2>
      <div className="folder-tree">
        {loadError && <p className="tree-error">{loadError}</p>}
        <ul className="tree-root">
          {drives.map((drive) => (
            <TreeNode
              key={drive.path}
              name={drive.name}
              path={drive.path}
              selectedPath={selectedPath}
              onSelect={setSelectedPath}
            />
          ))}
        </ul>
      </div>
      <div className="folder-tree-footer">
        <span className="selected-path">{selectedPath ?? "No folder selected"}</span>
        <button
          disabled={!selectedPath}
          onClick={() => selectedPath && onConfirm(selectedPath)}
        >
          Browse this folder
        </button>
      </div>
    </div>
  );
}
