//! Live updates: watch screenshot folders and tell the UI when images appear, move or vanish.

use notify::{event::ModifyKind, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{path::Path, sync::Mutex};
use tauri::{AppHandle, Emitter};

static WATCHER: Mutex<Option<RecommendedWatcher>> = Mutex::new(None);

/// Replace the watched set with `dirs`. The UI debounces the "shots-changed" event and rescans.
pub fn watch(app: AppHandle, dirs: Vec<String>) -> Result<(), String> {
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let relevant = matches!(ev.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)));
        if relevant && ev.paths.iter().any(|p| crate::scan::is_image(p)) {
            let _ = app.emit("shots-changed", ());
        }
    })
    .map_err(|e| e.to_string())?;
    for d in &dirs {
        let p = Path::new(d);
        if p.is_dir() {
            let _ = w.watch(p, RecursiveMode::Recursive);
        }
    }
    *WATCHER.lock().unwrap() = Some(w); // dropping the old watcher stops it
    Ok(())
}
