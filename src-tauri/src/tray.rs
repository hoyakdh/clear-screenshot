//! Menu-bar (status bar) icon with a small menu. The window can be closed while the app keeps
//! running here — it still watches folders and analyzes new screenshots.

use serde::Deserialize;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Wry,
};

const TRAY_ID: &str = "main";

/// Menu texts, sent by the UI in the user's language (Korean defaults until it does).
#[derive(Deserialize, Clone)]
pub struct TrayLabels {
    pub status: String,
    pub open: String,
    pub tidy: String,
    pub search: String,
    pub settings: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self {
            status: "Clear Screenshot".into(),
            open: "Clear Screenshot 열기".into(),
            tidy: "빠른 정리".into(),
            search: "검색…".into(),
            settings: "설정…".into(),
            quit: "종료".into(),
        }
    }
}

fn build_menu(app: &AppHandle, l: &TrayLabels) -> tauri::Result<Menu<Wry>> {
    let status = MenuItem::with_id(app, "status", &l.status, false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", &l.open, true, None::<&str>)?;
    let tidy = MenuItem::with_id(app, "quick_tidy", &l.tidy, true, None::<&str>)?;
    let search = MenuItem::with_id(app, "search", &l.search, true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", &l.settings, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", &l.quit, true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    Menu::with_items(app, &[&status, &sep()?, &open, &tidy, &search, &sep()?, &settings, &sep()?, &quit])
}

/// Bring the main window back (it may be hidden after closing it).
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &TrayLabels::default())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
        .icon_as_template(true) // macOS tints it for light/dark menu bars
        .tooltip("Clear Screenshot")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            action => {
                // Actions run in the window: show it, then let the UI handle the action.
                show_main(app);
                let _ = app.emit("tray-action", action.to_string());
            }
        })
        .build(app)?;
    Ok(())
}

/// Replace the menu texts (language change) and the status line (counts / progress).
pub fn update(app: &AppHandle, labels: &TrayLabels) -> Result<(), String> {
    let tray = app.tray_by_id(TRAY_ID).ok_or("no tray")?;
    let menu = build_menu(app, labels).map_err(|e| e.to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    tray.set_tooltip(Some(&labels.status)).map_err(|e| e.to_string())
}
