use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};

/// Extensions we're willing to write rating metadata directly into (well-supported by exiftool
/// without risk of corrupting the original). Everything else gets an XMP sidecar instead.
const EMBEDDABLE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "tiff", "tif"];

pub fn exiftool_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let relative = "exiftool-windows/exiftool.exe";
    #[cfg(target_os = "macos")]
    let relative = "exiftool-macos/exiftool";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let relative = "exiftool-linux/exiftool";

    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join(relative);
        if bundled.exists() {
            return Ok(bundled);
        }
    }

    // Dev fallback: run straight out of src-tauri/vendor.
    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor")
        .join(relative);
    if dev_path.exists() {
        return Ok(dev_path);
    }

    Err(format!(
        "exiftool binary not found (expected at {})",
        dev_path.display()
    ))
}

fn sidecar_path(photo: &Path) -> PathBuf {
    photo.with_extension("xmp")
}

fn is_embeddable(photo: &Path) -> bool {
    photo
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EMBEDDABLE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Reads the current star rating (0-5) for a batch of photos in a single exiftool invocation.
/// Prefers an XMP sidecar's Rating over the file's own embedded rating when a sidecar exists,
/// matching the convention used by Lightroom/Bridge.
pub fn read_ratings_batch(app: &AppHandle, photos: &[PathBuf]) -> HashMap<String, i32> {
    let mut result = HashMap::new();
    if photos.is_empty() {
        return result;
    }

    let Ok(exiftool) = exiftool_path(app) else {
        return result;
    };

    // Map "the file exiftool should actually read" -> "the catalog path it belongs to".
    let mut src_to_photo: HashMap<String, String> = HashMap::new();
    let mut sources: Vec<String> = Vec::new();
    for photo in photos {
        let sidecar = sidecar_path(photo);
        let source = if sidecar.exists() { sidecar } else { photo.clone() };
        let source_str = source.to_string_lossy().to_string();
        src_to_photo.insert(source_str.clone(), photo.to_string_lossy().to_string());
        sources.push(source_str);
    }

    let output = Command::new(&exiftool)
        .args(["-j", "-Rating", "-XMP:Rating"])
        .args(&sources)
        .output();

    let Ok(output) = output else {
        return result;
    };
    let Ok(parsed) = serde_json::from_slice::<Vec<Value>>(&output.stdout) else {
        return result;
    };

    for entry in parsed {
        let Some(source_file) = entry.get("SourceFile").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(photo_path) = src_to_photo.get(source_file) else {
            continue;
        };
        let rating = entry
            .get("Rating")
            .or_else(|| entry.get("XMP:Rating"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        result.insert(photo_path.clone(), rating);
    }

    result
}

/// Writes a 0-5 star rating for a single photo, embedding it directly for well-supported
/// raster formats, or into an XMP sidecar file for everything else (RAW, PNG, HEIC, ...).
#[tauri::command]
pub fn set_rating(app: AppHandle, path: String, rating: i32) -> Result<(), String> {
    let rating = rating.clamp(0, 5);
    let exiftool = exiftool_path(&app)?;
    let photo = Path::new(&path);
    let rating_arg = format!("-Rating={}", rating);
    let xmp_rating_arg = format!("-XMP:Rating={}", rating);

    let status = if is_embeddable(photo) {
        Command::new(&exiftool)
            .args(["-overwrite_original", &rating_arg, &xmp_rating_arg])
            .arg(&path)
            .status()
    } else {
        let sidecar = sidecar_path(photo);
        if sidecar.exists() {
            Command::new(&exiftool)
                .args(["-overwrite_original", &rating_arg])
                .arg(&sidecar)
                .status()
        } else {
            Command::new(&exiftool)
                .arg("-o")
                .arg(&sidecar)
                .arg(&rating_arg)
                .status()
        }
    };

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("exiftool exited with status {}", s)),
        Err(e) => Err(e.to_string()),
    }
}
