export interface DriveEntry {
  name: string;
  path: string;
}

export interface DirEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

export interface PhotoRecord {
  path: string;
  raw_path: string | null;
  filename: string;
  mtime: number;
  width: number | null;
  height: number | null;
  rating: number;
  thumbnail: string | null;
}

export interface IndexProgress {
  done: number;
  total: number;
}

// Which file(s) to hand to the OS when dragging a RAW+JPG pair out of the app.
export type DragMode = "jpg" | "raw" | "both";

export function dragItemFor(photo: PhotoRecord, mode: DragMode): string[] {
  if (!photo.raw_path) return [photo.path];
  switch (mode) {
    case "raw":
      return [photo.raw_path];
    case "both":
      return [photo.path, photo.raw_path];
    default:
      return [photo.path];
  }
}
