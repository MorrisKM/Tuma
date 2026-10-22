//! The desktop shell: Tauri commands that call into `tuma_core`. Keep logic out of here.

#[tauri::command]
fn app_version() -> &'static str {
    tuma_core::VERSION
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
