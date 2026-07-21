mod fs_browser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            fs_browser::list_drives,
            fs_browser::list_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
