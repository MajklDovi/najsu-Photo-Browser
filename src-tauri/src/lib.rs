mod ai;
mod analysis;
mod catalog;
mod exiftool;
mod fs_browser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_drag::init())
        .invoke_handler(tauri::generate_handler![
            fs_browser::list_drives,
            fs_browser::list_dir,
            catalog::check_catalog,
            catalog::load_catalog,
            catalog::index_folder,
            exiftool::set_rating,
            analysis::analyze_folder,
            analysis::load_analysis,
            analysis::apply_auto_ratings,
            ai::ai_check,
            ai::ai_review,
            ai::ai_cancel,
            ai::ai_rank_series,
            ai::load_series_ai,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
