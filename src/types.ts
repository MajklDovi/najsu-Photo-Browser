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
