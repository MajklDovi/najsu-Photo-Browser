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


/// Builds an exiftool command that doesn't flash a console window on Windows.
pub fn exiftool_command(exiftool: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(exiftool);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Runs exiftool with `-j` over many files, splitting them into several invocations so the
/// command line never exceeds Windows' ~32k character limit. Returns the merged JSON entries.
pub fn run_json(exiftool: &Path, base_args: &[&str], files: &[String]) -> Vec<Value> {
    const MAX_CMDLINE: usize = 20_000;
    let mut out = Vec::new();
    let mut chunk: Vec<&String> = Vec::new();
    let mut len = 0usize;

    let run = |chunk: &mut Vec<&String>, out: &mut Vec<Value>| {
        if chunk.is_empty() {
            return;
        }
        if let Ok(output) = exiftool_command(exiftool).args(base_args).args(chunk.iter()).output() {
            if let Ok(mut parsed) = serde_json::from_slice::<Vec<Value>>(&output.stdout) {
                out.append(&mut parsed);
            }
        }
        chunk.clear();
    };

    for file in files {
        if len + file.len() + 3 > MAX_CMDLINE {
            run(&mut chunk, &mut out);
            len = 0;
        }
        len += file.len() + 3;
        chunk.push(file);
    }
    run(&mut chunk, &mut out);
    out
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

    let parsed = run_json(
        &exiftool,
        &["-charset", "filename=utf8", "-j", "-Rating", "-XMP:Rating"],
        &sources,
    );

    for entry in parsed {
        let Some(source_file) = entry.get("SourceFile").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(photo_path) = src_to_photo.get(source_file) else {
            continue;
        };
        // Only files that actually carry a rating get an entry: absent means "unrated".
        if let Some(rating) = entry
            .get("Rating")
            .or_else(|| entry.get("XMP:Rating"))
            .and_then(|v| v.as_i64())
        {
            result.insert(photo_path.clone(), rating as i32);
        }
    }

    result
}

fn write_rating_for(exiftool: &Path, photo: &Path, rating: i32) -> Result<(), String> {
    let rating_arg = format!("-Rating={}", rating);
    let xmp_rating_arg = format!("-XMP:Rating={}", rating);

    let status = if is_embeddable(photo) {
        exiftool_command(exiftool)
            .args(["-charset", "filename=utf8", "-overwrite_original", &rating_arg, &xmp_rating_arg])
            .arg(photo)
            .status()
    } else {
        let sidecar = sidecar_path(photo);
        if sidecar.exists() {
            exiftool_command(exiftool)
                .args(["-charset", "filename=utf8", "-overwrite_original", &rating_arg])
                .arg(&sidecar)
                .status()
        } else {
            exiftool_command(exiftool)
                .args(["-charset", "filename=utf8"])
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

/// Writes a 0-5 star rating for a photo, embedding it directly for well-supported raster
/// formats, or into an XMP sidecar file for everything else (RAW, PNG, HEIC, ...). When
/// `raw_path` is given (a paired RAW file sharing the same filename stem), its rating is
/// written too so both files agree - see `catalog::group_into_units`.
#[tauri::command]
pub fn set_rating(
    app: AppHandle,
    folder: String,
    path: String,
    rating: i32,
    raw_path: Option<String>,
) -> Result<(), String> {
    let rating = rating.clamp(0, 5);
    write_rating_files(&app, &path, raw_path.as_deref(), rating)?;
    crate::analysis::record_rating(&app, &folder, &path, rating);
    Ok(())
}

/// Writes the rating to the photo (and its paired RAW) without touching the catalogue.
pub fn write_rating_files(
    app: &AppHandle,
    path: &str,
    raw_path: Option<&str>,
    rating: i32,
) -> Result<(), String> {
    let exiftool = exiftool_path(app)?;
    write_rating_for(&exiftool, Path::new(path), rating)?;
    if let Some(raw) = raw_path {
        write_rating_for(&exiftool, Path::new(raw), rating)?;
    }
    Ok(())
}
