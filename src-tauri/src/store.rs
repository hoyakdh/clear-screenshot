//! Small persistent app state: screenshots the user chose to keep, and settings.

use std::{fs, path::PathBuf};

fn data_file(name: &str) -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("clear-screenshot").join(name)
}

fn kept_file() -> PathBuf {
    data_file("kept.json")
}

pub fn load_kept() -> Vec<String> {
    fs::read_to_string(kept_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Atomic write (temp file + rename) so a crash never leaves a half-written file.
fn write_json(file: PathBuf, value: &impl serde::Serialize) -> Result<(), String> {
    let dir = file.parent().expect("has parent");
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

pub fn save_kept(kept: &[String]) -> Result<(), String> {
    write_json(kept_file(), &kept)
}

/// Manual category corrections: { "<path>": { "add": [keys], "remove": [keys] } }.
pub fn load_overrides() -> serde_json::Value {
    fs::read_to_string(data_file("overrides.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub fn save_overrides(v: &serde_json::Value) -> Result<(), String> {
    write_json(data_file("overrides.json"), v)
}

/// Settings are owned by the UI; the backend just stores the JSON object.
pub fn load_settings() -> serde_json::Value {
    fs::read_to_string(data_file("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub fn save_settings(settings: &serde_json::Value) -> Result<(), String> {
    write_json(data_file("settings.json"), settings)
}
