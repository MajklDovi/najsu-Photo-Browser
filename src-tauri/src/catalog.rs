use base64::Engine;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Emitter, Manager};

const THUMBNAIL_MAX_DIM: u32 = 320;
const RASTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp"];
const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "pef", "srw",
    "x3f", "kdc", "dcr", "mrw", "raw",
];

#[derive(Serialize, Clone)]
pub struct PhotoRecord {
    pub path: String,
    /// Path of a paired RAW file sharing the same filename stem, if one was found
    /// alongside `path` (e.g. IMG_0001.CR2 next to IMG_0001.JPG). Rating operations
    /// apply to both files when this is set.
    pub raw_path: Option<String>,
    pub filename: String,
    pub mtime: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub rating: i32,
    /// base64 data URL, e.g. "data:image/jpeg;base64,..."; None if no thumbnail could be generated
    pub thumbnail: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct IndexProgress {
    pub done: usize,
    pub total: usize,
}

struct Unit {
    primary: PathBuf,
    raw: Option<PathBuf>,
}

fn catalog_db_path(app: &AppHandle, folder: &str) -> Result<PathBuf, String> {
    let mut hasher = Sha256::new();
    hasher.update(folder.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let hash = &hash[..24];

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("catalogs");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    Ok(data_dir.join(format!("{}.db", hash)))
}

fn open_db(app: &AppHandle, folder: &str) -> Result<Connection, String> {
    let db_path = catalog_db_path(app, folder)?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS photos (
            path TEXT PRIMARY KEY,
            raw_path TEXT,
            filename TEXT NOT NULL,
            mtime INTEGER NOT NULL,
            width INTEGER,
            height INTEGER,
            rating INTEGER NOT NULL DEFAULT 0,
            thumbnail BLOB,
            indexed_at INTEGER NOT NULL
        );",
    )
    .map_err(|e| e.to_string())?;
    // Older catalogs created before RAW+JPEG pairing won't have this column yet.
    let _ = conn.execute("ALTER TABLE photos ADD COLUMN raw_path TEXT", []);
    Ok(conn)
}

fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<PhotoRecord> {
    let thumb: Option<Vec<u8>> = row.get("thumbnail")?;
    Ok(PhotoRecord {
        path: row.get("path")?,
        raw_path: row.get("raw_path")?,
        filename: row.get("filename")?,
        mtime: row.get("mtime")?,
        width: row.get("width")?,
        height: row.get("height")?,
        rating: row.get("rating")?,
        thumbnail: thumb.map(|bytes| {
            format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        }),
    })
}

/// True if a catalog already exists for this folder and has at least one entry.
#[tauri::command]
pub fn check_catalog(app: AppHandle, folder: String) -> Result<bool, String> {
    let db_path = catalog_db_path(&app, &folder)?;
    if !db_path.exists() {
        return Ok(false);
    }
    let conn = open_db(&app, &folder)?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

/// Load a previously indexed catalog instantly, with no re-scan of the filesystem.
#[tauri::command]
pub fn load_catalog(app: AppHandle, folder: String) -> Result<Vec<PhotoRecord>, String> {
    let conn = open_db(&app, &folder)?;
    let mut stmt = conn
        .prepare("SELECT * FROM photos ORDER BY filename COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_record)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase())
}

fn is_raster_file(path: &Path) -> bool {
    extension_lower(path)
        .map(|e| RASTER_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or(false)
}

fn is_raw_file(path: &Path) -> bool {
    extension_lower(path)
        .map(|e| RAW_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or(false)
}

fn make_thumbnail(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::open(path).ok()?;
    let (width, height) = (img.width(), img.height());
    let thumb = img.thumbnail(THUMBNAIL_MAX_DIM, THUMBNAIL_MAX_DIM);
    let mut bytes: Vec<u8> = Vec::new();
    thumb
        .to_rgb8()
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some((bytes, width, height))
}

/// Groups files sharing the same directory and filename stem: a lone raster + a lone RAW
/// become one paired unit (raster as the primary/thumbnail source), everything else stays
/// standalone. Grouping is scoped per-directory so recursive indexing never pairs files
/// from unrelated folders that happen to share a name.
fn group_into_units(files: &[PathBuf]) -> Vec<Unit> {
    let mut by_stem: HashMap<(PathBuf, String), Vec<PathBuf>> = HashMap::new();
    for file in files {
        let dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        by_stem.entry((dir, stem)).or_default().push(file.clone());
    }

    let mut units = Vec::new();
    for (_, group) in by_stem {
        let rasters: Vec<&PathBuf> = group.iter().filter(|p| is_raster_file(p)).collect();
        let raws: Vec<&PathBuf> = group.iter().filter(|p| is_raw_file(p)).collect();

        if rasters.len() == 1 && raws.len() == 1 {
            units.push(Unit {
                primary: rasters[0].clone(),
                raw: Some(raws[0].clone()),
            });
        } else {
            for file in group {
                units.push(Unit { primary: file, raw: None });
            }
        }
    }
    units
}

/// Walk `folder` (its direct contents, or every descendant when `recursive` is set),
/// generate thumbnails for anything not already indexed at its current mtime, drop rows
/// for files that no longer exist, and emit progress events as `index-progress` / a photo
/// event per file as `index-photo`, finishing with `index-complete`. Runs on a background
/// thread so the UI stays responsive.
#[tauri::command]
pub fn index_folder(app: AppHandle, window: tauri::Window, folder: String, recursive: bool) {
    std::thread::spawn(move || {
        if let Err(e) = run_index(&app, &window, &folder, recursive) {
            let _ = window.emit("index-error", e);
        }
    });
}

fn collect_files(folder: &str, recursive: bool) -> Result<Vec<PathBuf>, String> {
    if recursive {
        Ok(walkdir::WalkDir::new(folder)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.into_path())
            .filter(|p| p.is_file() && (is_raster_file(p) || is_raw_file(p)))
            .collect())
    } else {
        Ok(std::fs::read_dir(folder)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && (is_raster_file(p) || is_raw_file(p)))
            .collect())
    }
}

fn run_index(app: &AppHandle, window: &tauri::Window, folder: &str, recursive: bool) -> Result<(), String> {
    let mut conn = open_db(app, folder)?;

    let files = collect_files(folder, recursive)?;

    let units = group_into_units(&files);
    let total = units.len();
    let _ = window.emit("index-progress", IndexProgress { done: 0, total });

    let current_paths: std::collections::HashSet<String> = units
        .iter()
        .map(|u| u.primary.to_string_lossy().to_string())
        .collect();

    {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx.prepare("SELECT path FROM photos").map_err(|e| e.to_string())?;
            let existing: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            for path in existing {
                if !current_paths.contains(&path) {
                    tx.execute("DELETE FROM photos WHERE path = ?1", [&path])
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    let rating_lookup_paths: Vec<PathBuf> = units.iter().map(|u| u.primary.clone()).collect();
    let ratings = crate::exiftool::read_ratings_batch(app, &rating_lookup_paths);

    for (i, unit) in units.iter().enumerate() {
        let path = &unit.primary;
        let path_str = path.to_string_lossy().to_string();
        let raw_path_str = unit.raw.as_ref().map(|p| p.to_string_lossy().to_string());
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|t| {
                t.duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let existing_mtime: Option<i64> = conn
            .query_row(
                "SELECT mtime FROM photos WHERE path = ?1",
                [&path_str],
                |r| r.get(0),
            )
            .ok();

        let rating = ratings.get(&path_str).copied().unwrap_or(0);

        let mut record = if existing_mtime == Some(mtime) {
            let mut stmt = conn
                .prepare("SELECT * FROM photos WHERE path = ?1")
                .map_err(|e| e.to_string())?;
            let mut record = stmt
                .query_row([&path_str], row_to_record)
                .map_err(|e| e.to_string())?;
            if record.raw_path != raw_path_str {
                record.raw_path = raw_path_str.clone();
                conn.execute(
                    "UPDATE photos SET raw_path = ?1 WHERE path = ?2",
                    rusqlite::params![raw_path_str, path_str],
                )
                .map_err(|e| e.to_string())?;
            }
            record
        } else {
            let thumb = make_thumbnail(path);
            let (thumb_bytes, width, height) = match &thumb {
                Some((bytes, w, h)) => (Some(bytes.clone()), Some(*w), Some(*h)),
                None => (None, None, None),
            };
            let now = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            conn.execute(
                "INSERT INTO photos (path, raw_path, filename, mtime, width, height, rating, thumbnail, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(path) DO UPDATE SET
                    raw_path = excluded.raw_path,
                    filename = excluded.filename,
                    mtime = excluded.mtime,
                    width = excluded.width,
                    height = excluded.height,
                    rating = excluded.rating,
                    thumbnail = excluded.thumbnail,
                    indexed_at = excluded.indexed_at",
                rusqlite::params![
                    path_str,
                    raw_path_str,
                    filename,
                    mtime,
                    width,
                    height,
                    rating,
                    thumb_bytes,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;

            PhotoRecord {
                path: path_str.clone(),
                raw_path: raw_path_str.clone(),
                filename,
                mtime,
                width,
                height,
                rating,
                thumbnail: thumb_bytes.map(|bytes| {
                    format!(
                        "data:image/jpeg;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    )
                }),
            }
        };

        if record.rating != rating {
            record.rating = rating;
            conn.execute(
                "UPDATE photos SET rating = ?1 WHERE path = ?2",
                rusqlite::params![rating, path_str],
            )
            .map_err(|e| e.to_string())?;
        }

        let _ = window.emit("index-photo", record);
        let _ = window.emit(
            "index-progress",
            IndexProgress {
                done: i + 1,
                total,
            },
        );
    }

    let _ = window.emit("index-complete", total);
    Ok(())
}
