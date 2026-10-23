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
  // True only when the file carries an explicit 0-5 rating; false means "never rated".
  rated: boolean;
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

// Raw local-analysis result for one photo (each category scored 0-5).
export interface AnalysisRecord {
  path: string;
  sharpness: number;
  shake: number;
  exposure: number;
  noise: number;
  // null when the photo has no clear horizon or vertical lines to measure
  horizon: number | null;
  horizon_deg: number | null;
  series_id: string | null;
  // Local AI review (null until the photo has been reviewed)
  ai: AiResult | null;
}

export interface AiResult {
  model: string;
  composition: number;
  subject: number;
  // null when nobody is visible in the photo
  people: number | null;
  impact: number;
  summary: string;
}

// The local AI's side-by-side verdict on one series: frame paths best first.
export interface SeriesAi {
  series_id: string;
  ranking: string[];
  // Points per frame, aligned with `ranking`. Equal points mean the model had no clear winner.
  points: number[];
  reason: string;
  model: string;
  // 1 = one ranking pass, 2 = also re-asked with the frames in reverse order
  passes: number;
}
