import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DirEntry } from "../types";
import "./SubfolderBar.css";

interface SubfolderBarProps {
  root: string;
  currentFolder: string;
  onNavigate: (path: string) => void;
}

function pathSeparator(path: string) {
  return path.includes("\\") ? "\\" : "/";
}

function breadcrumbSegments(root: string, currentFolder: string) {
  const sep = pathSeparator(root);
  const relative = currentFolder.slice(root.length).replace(/^[\\/]+/, "");
  const parts = relative.length > 0 ? relative.split(/[\\/]+/) : [];

  const segments = [{ label: root, path: root }];
  let acc = root;
  for (const part of parts) {
    acc = acc.endsWith(sep) ? acc + part : acc + sep + part;
    segments.push({ label: part, path: acc });
  }
  return segments;
}

export function SubfolderBar({ root, currentFolder, onNavigate }: SubfolderBarProps) {
  const [subfolders, setSubfolders] = useState<DirEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<DirEntry[]>("list_dir", { path: currentFolder })
      .then((entries) => {
        if (!cancelled) setSubfolders(entries.filter((e) => e.is_dir));
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [currentFolder]);

  const crumbs = breadcrumbSegments(root, currentFolder);

  return (
    <div className="subfolder-bar">
      <div className="breadcrumb">
        {crumbs.map((crumb, i) => (
          <span key={crumb.path} className="breadcrumb-item">
            {i > 0 && <span className="breadcrumb-sep">›</span>}
            <button
              className={`breadcrumb-button${crumb.path === currentFolder ? " breadcrumb-current" : ""}`}
              onClick={() => onNavigate(crumb.path)}
              disabled={crumb.path === currentFolder}
            >
              {crumb.label}
            </button>
          </span>
        ))}
      </div>
      {error && <p className="tree-error">{error}</p>}
      {subfolders.length > 0 && (
        <div className="subfolder-list">
          {subfolders.map((sub) => (
            <button
              key={sub.path}
              className="subfolder-chip"
              onClick={() => onNavigate(sub.path)}
              title={sub.path}
            >
              📁 {sub.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
