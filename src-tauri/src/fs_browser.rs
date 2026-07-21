use serde::Serialize;
use std::fs;
use std::path::Path;
use sysinfo::Disks;

#[derive(Serialize, Clone)]
pub struct DriveEntry {
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// Top-level roots to seed the tree/browse view: all mounted disks/volumes.
#[tauri::command]
pub fn list_drives() -> Vec<DriveEntry> {
    let disks = Disks::new_with_refreshed_list();
    disks
        .iter()
        .map(|d| {
            let path = d.mount_point().to_string_lossy().to_string();
            let label = d.name().to_string_lossy().to_string();
            let name = if label.trim().is_empty() {
                path.clone()
            } else {
                format!("{} ({})", label, path)
            };
            DriveEntry { name, path }
        })
        .collect()
}

/// List immediate subdirectories (and files, for context) of a given folder.
/// Entries the OS denies access to are silently skipped rather than failing the whole listing.
#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let dir = Path::new(&path);
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;

    let mut entries: Vec<DirEntry> = read_dir
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            Some(DirEntry {
                name,
                path: entry.path().to_string_lossy().to_string(),
                is_dir: file_type.is_dir(),
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}
