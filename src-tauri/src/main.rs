#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod classify;
mod device;
#[cfg(target_os = "macos")]
mod glass;
mod ops;
mod photos;
mod scan;
mod store;
mod tray;
mod vision;
mod watch;

use rayon::prelude::*;
use std::process::Command;
use tauri::async_runtime::spawn_blocking;

fn join_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
async fn scan(
    whole_mac: Option<bool>,
    extra_dirs: Option<Vec<String>>,
    capture_dirs: Option<Vec<String>>,
) -> Result<scan::ScanResult, String> {
    let dirs = extra_dirs.unwrap_or_default();
    let cap = capture_dirs.unwrap_or_default();
    spawn_blocking(move || scan::scan(whole_mac.unwrap_or(true), &dirs, &cap)).await.map_err(join_err)
}

#[tauri::command]
fn detect_capture_dirs() -> Vec<String> {
    scan::detect_capture_dirs()
}

#[tauri::command]
fn watch_dirs(app: tauri::AppHandle, dirs: Vec<String>) -> Result<(), String> {
    watch::watch(app, dirs)
}

#[tauri::command]
async fn photos_scan() -> Result<photos::PhotosScan, String> {
    spawn_blocking(photos::scan).await.map_err(join_err)
}

#[tauri::command]
async fn preview_path(path: String) -> Result<String, String> {
    spawn_blocking(move || ops::preview_path(&path)).await.map_err(join_err)?
}

#[tauri::command]
async fn thumbnails(paths: Vec<String>) -> Result<Vec<ops::Thumb>, String> {
    spawn_blocking(move || paths.par_iter().map(|p| ops::thumb(p)).collect())
        .await
        .map_err(join_err)
}

#[tauri::command]
async fn analyze(paths: Vec<String>, ai: Option<bool>) -> Result<Vec<ops::Analysis>, String> {
    let want_ai = ai.unwrap_or(true);
    spawn_blocking(move || paths.par_iter().map(|p| ops::analyze(p, want_ai)).collect())
        .await
        .map_err(join_err)
}

#[tauri::command]
async fn similar(paths: Vec<String>, times: Option<Vec<i64>>, level: Option<String>) -> Result<Vec<Vec<String>>, String> {
    let times = times.unwrap_or_default();
    let level = level.unwrap_or_else(|| "normal".into());
    spawn_blocking(move || ops::similar_groups(&paths, &times, &level)).await.map_err(join_err)
}

#[tauri::command]
async fn trash(paths: Vec<String>) -> Result<Vec<String>, String> {
    spawn_blocking(move || ops::trash(&paths)).await.map_err(join_err)?
}

#[tauri::command]
async fn organize(paths: Vec<String>, root: String, layout: Option<String>) -> Result<ops::OrganizeResult, String> {
    let layout = layout.unwrap_or_else(|| "month".into());
    spawn_blocking(move || ops::organize(&paths, &root, &layout)).await.map_err(join_err)
}

/// Which of `paths` still need analysis (the rest are already stored and load instantly).
#[tauri::command]
async fn pending_analysis(paths: Vec<String>, ai: Option<bool>) -> Result<Vec<String>, String> {
    let want_ai = ai.unwrap_or(true);
    spawn_blocking(move || paths.into_iter().filter(|p| ops::needs_analysis(p, want_ai)).collect())
        .await
        .map_err(join_err)
}

/// Update the menu-bar menu texts and status line (sent by the UI in the user's language).
#[tauri::command]
fn update_tray(app: tauri::AppHandle, labels: tray::TrayLabels) -> Result<(), String> {
    tray::update(&app, &labels)
}

/// Whether Apple's on-device model can describe pictures on this Mac.
#[tauri::command]
async fn ai_available() -> bool {
    spawn_blocking(ai::available).await.unwrap_or(false)
}

#[tauri::command]
fn load_overrides() -> serde_json::Value {
    store::load_overrides()
}

#[tauri::command]
fn save_overrides(overrides: serde_json::Value) -> Result<(), String> {
    store::save_overrides(&overrides)
}

#[tauri::command]
fn load_settings() -> serde_json::Value {
    store::load_settings()
}

#[tauri::command]
fn save_settings(settings: serde_json::Value) -> Result<(), String> {
    store::save_settings(&settings)
}

/// Native Finder folder picker. Returns None if cancelled.
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle, start: Option<String>, title: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    spawn_blocking(move || {
        let mut dlg = app.dialog().file().set_title(title.unwrap_or_else(|| "Clear Screenshot".into()));
        if let Some(s) = start {
            dlg = dlg.set_directory(s);
        }
        dlg.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// "system" | "light" | "dark" — also switches the native glass/vibrancy appearance.
#[tauri::command]
fn set_theme(window: tauri::WebviewWindow, theme: String) -> Result<(), String> {
    let t = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    window.set_theme(t).map_err(join_err)
}

#[tauri::command]
fn cache_info() -> ops::CacheInfo {
    ops::cache_info()
}

#[tauri::command]
async fn clear_cache(what: String) -> Result<(), String> {
    spawn_blocking(move || ops::clear_cache(&what)).await.map_err(join_err)?
}

/// Where macOS currently saves new screenshots.
#[tauri::command]
fn screencapture_location() -> String {
    scan::screenshot_dir().to_string_lossy().into_owned()
}

/// Point macOS's own screenshot location at `path` (same as Screenshot app ▸ Options ▸ Save to).
#[tauri::command]
fn set_screencapture_location(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err("E:folder_missing".into());
    }
    let ok = Command::new("/usr/bin/defaults")
        .args(["write", "com.apple.screencapture", "location", "-string", &path])
        .status()
        .map_err(join_err)?
        .success();
    if !ok {
        return Err("E:macos_setting_failed".into());
    }
    let _ = Command::new("/usr/bin/killall").arg("SystemUIServer").status();
    Ok(())
}

#[tauri::command]
fn native_glass() -> bool {
    #[cfg(target_os = "macos")]
    return glass::active();
    #[cfg(not(target_os = "macos"))]
    false
}

#[tauri::command]
fn load_kept() -> Vec<String> {
    store::load_kept()
}

#[tauri::command]
fn save_kept(kept: Vec<String>) -> Result<(), String> {
    store::save_kept(&kept)
}

fn open(args: &[&str]) -> Result<(), String> {
    Command::new("/usr/bin/open").args(args).status().map(|_| ()).map_err(join_err)
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    if photos::is_photo(&path) {
        return open(&["-b", "com.apple.Photos"]);
    }
    open(&["-R", &path])
}

#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    if photos::is_photo(&path) {
        return open(&["-b", "com.apple.Photos"]);
    }
    open(&[&path])
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                glass::install(&w);
            }
            tray::install(app.handle())?;
            Ok(())
        })
        // Closing the window keeps the app in the menu bar (watching and analyzing);
        // "종료" in the menu-bar menu quits.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            scan, photos_scan, preview_path, thumbnails, analyze, similar, trash, organize, reveal, open_file, load_kept, save_kept, native_glass,
            load_settings, save_settings, pick_folder, set_theme, cache_info, clear_cache,
            screencapture_location, set_screencapture_location, detect_capture_dirs, watch_dirs, ai_available, load_overrides, save_overrides, pending_analysis, update_tray
        ])
        .build(tauri::generate_context!())
        .expect("error while building Clear Screenshot")
        .run(|app, event| {
            // Clicking the Dock icon brings a hidden window back.
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_main(app);
            }
        });
}
