import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { IndexProgress, PhotoRecord } from "../types";

export function usePhotoCatalog(folder: string) {
  const [photos, setPhotos] = useState<PhotoRecord[]>([]);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [indexing, setIndexing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const photosByPath = useRef<Map<string, PhotoRecord>>(new Map());

  const applyPhoto = useCallback((photo: PhotoRecord) => {
    photosByPath.current.set(photo.path, photo);
    const sorted = Array.from(photosByPath.current.values()).sort((a, b) =>
      a.filename.localeCompare(b.filename, undefined, { sensitivity: "base" }),
    );
    setPhotos(sorted);
  }, []);

  const startIndexing = useCallback(async () => {
    setIndexing(true);
    setError(null);
    await invoke("index_folder", { folder });
  }, [folder]);

  const load = useCallback(async () => {
    photosByPath.current.clear();
    setPhotos([]);
    setProgress(null);
    setError(null);

    const hasCatalog = await invoke<boolean>("check_catalog", { folder });
    if (hasCatalog) {
      const cached = await invoke<PhotoRecord[]>("load_catalog", { folder });
      cached.forEach((p) => photosByPath.current.set(p.path, p));
      setPhotos(cached);
      return;
    }
    await startIndexing();
  }, [folder, startIndexing]);

  const refresh = useCallback(async () => {
    await startIndexing();
  }, [startIndexing]);

  const setRating = useCallback(
    async (path: string, rating: number) => {
      const previous = photosByPath.current.get(path);
      if (previous) {
        applyPhoto({ ...previous, rating });
      }
      try {
        await invoke("set_rating", { path, rating });
      } catch (e) {
        if (previous) applyPhoto(previous);
        setError(String(e));
      }
    },
    [applyPhoto],
  );

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    const unlistenPhoto = listen<PhotoRecord>("index-photo", (event) => {
      applyPhoto(event.payload);
    });
    const unlistenProgress = listen<IndexProgress>("index-progress", (event) => {
      setProgress(event.payload);
    });
    const unlistenComplete = listen<number>("index-complete", () => {
      setIndexing(false);
      setProgress(null);
    });
    const unlistenError = listen<string>("index-error", (event) => {
      setIndexing(false);
      setError(event.payload);
    });

    return () => {
      unlistenPhoto.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenComplete.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, [applyPhoto]);

  return { photos, progress, indexing, error, refresh, setRating };
}
