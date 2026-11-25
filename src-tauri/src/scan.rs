//! Locating screenshots on macOS.
//!
//! Two sources are merged:
//! 1. Spotlight: every file tagged `kMDItemIsScreenCapture == 1`, wherever it lives.
//! 2. The configured screenshot folder (`defaults read com.apple.screencapture location`),
//!    walked a few levels deep for files whose names look like screenshots. This catches
//!    files Spotlight has not indexed yet, or that lost the metadata tag.

use serde::Serialize;
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::UNIX_EPOCH,
};
use unicode_normalization::UnicodeNormalization;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "heic", "tif", "tiff"];
const NAME_PREFIXES: &[&str] = &["스크린샷", "Screenshot", "Screen Shot", "CleanShot"];
const WALK_DEPTH: usize = 3;

#[derive(Serialize, Clone)]
pub struct Shot {
    pub path: String,
    pub name: String,
    pub size: u64,
    /// Unix seconds (creation time, falling back to modification time).
    pub created: i64,
    /// Pixel size (0 if unknown).
    pub width: u32,
    pub height: u32,
    /// Which device took it (see `device::detect`).
    pub device: crate::device::Device,
}

#[derive(Serialize)]
pub struct ScanResult {
    pub dir: String,
    pub home: String,
    pub items: Vec<Shot>,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// The folder macOS saves new screenshots into.
pub fn screenshot_dir() -> PathBuf {
    let home = home();
    if let Ok(out) = Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.screencapture", "location"])
        .output()
    {
        if out.status.success() {
            let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !raw.is_empty() {
                let p = match raw.strip_prefix('~') {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(raw),
                };
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    home.join("Desktop")
}

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn looks_like_screenshot(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // macOS may store Korean file names decomposed (NFD); compare in NFC.
    let name: String = name.nfc().collect();
    NAME_PREFIXES.iter().any(|pre| name.starts_with(pre))
}

/// Every file macOS tagged as a screen capture (i.e. taken on this Mac).
fn mac_captures() -> HashSet<PathBuf> {
    Command::new("/usr/bin/mdfind")
        .arg("kMDItemIsScreenCapture == 1")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// Whole-Mac Spotlight search: files tagged as screen captures, plus files merely named like
/// screenshots (e.g. ones copied/AirDropped from elsewhere that lost the tag).
fn spotlight() -> Vec<PathBuf> {
    let mut query = String::from("kMDItemIsScreenCapture == 1");
    for pre in NAME_PREFIXES {
        query.push_str(&format!(" || kMDItemFSName == \"{pre}*\"c"));
    }
    let Ok(out) = Command::new("/usr/bin/mdfind").arg(query).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|p| is_image(p) && !is_excluded(p))
        .collect()
}

/// Skip app data, hidden folders, bundles and the Photos library package (handled via PhotoKit).
fn is_excluded(p: &Path) -> bool {
    p.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.')
            || s == "Library"
            || s == "node_modules"
            || [".app", ".photoslibrary", ".photolibrary", ".bundle", ".framework"]
                .iter()
                .any(|ext| s.ends_with(ext))
    })
}

/// `any_name`: in a capture app's own folder every image counts, whatever it is called.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, any_name: bool) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(true)
        {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out, any_name);
            }
        } else if ft.is_file() && is_image(&p) && (any_name || looks_like_screenshot(&p)) {
            out.push(p);
        }
    }
}

/// `taken_on_mac`: tagged by macOS, or saved by a Mac capture app into one of its folders.
fn to_shot(p: &Path, taken_on_mac: &HashSet<PathBuf>) -> Option<Shot> {
    let meta = fs::metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    let t = meta.created().or_else(|_| meta.modified()).ok()?;
    let created = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // Reads only the image header. HEIC/TIFF are not supported here and stay 0×0.
    let (width, height) = image::image_dimensions(p).unwrap_or((0, 0));
    Some(Shot {
        path: p.to_string_lossy().into_owned(),
        name: p.file_name()?.to_string_lossy().nfc().collect(),
        size: meta.len(),
        created,
        width,
        height,
        device: crate::device::detect(width, height, taken_on_mac.contains(p)),
    })
}

/// `whole_mac`: also search everywhere via Spotlight. `extra_dirs`: more folders to walk
/// (e.g. a custom organize folder), in addition to the macOS screenshot folder.
/// `capture_dirs`: folders where another capture app saves (all images there are screenshots).
pub fn scan(whole_mac: bool, extra_dirs: &[String], capture_dirs: &[String]) -> ScanResult {
    let dir = screenshot_dir();
    let mut found: BTreeSet<PathBuf> = if whole_mac { spotlight().into_iter().collect() } else { BTreeSet::new() };
    let mut walked = Vec::new();
    walk(&dir, WALK_DEPTH, &mut walked, false);
    for extra in extra_dirs {
        let p = PathBuf::from(extra);
        if p.is_dir() && p != dir {
            walk(&p, WALK_DEPTH, &mut walked, false);
        }
    }
    let mut captured = Vec::new();
    for d in capture_dirs {
        let p = PathBuf::from(d);
        if p.is_dir() {
            walk(&p, WALK_DEPTH, &mut captured, true);
        }
    }
    found.extend(walked);
    found.extend(captured.iter().cloned());

    let mut on_mac = mac_captures();
    on_mac.extend(captured);
    let mut items: Vec<Shot> = found.iter().filter_map(|p| to_shot(p, &on_mac)).collect();
    items.sort_by(|a, b| b.created.cmp(&a.created));
    ScanResult {
        dir: dir.to_string_lossy().into_owned(),
        home: home().to_string_lossy().into_owned(),
        items,
    }
}

/// Folders that well-known capture apps save into by default and that exist on this Mac.
pub fn detect_capture_dirs() -> Vec<String> {
    let home = home();
    let names = [
        "oh-my-opensnap", "CleanShot", "CleanShot X", "Shottr", "Snagit", "Xnapper", "Monosnap",
        "Lightshot", "Snipaste", "PixPin", "Zight", "Skitch",
    ];
    let bases = [home.join("Desktop"), home.join("Pictures"), home.join("Documents"), home.join("Downloads"), home.clone()];
    let mut found = Vec::new();
    for base in &bases {
        for n in names {
            let p = base.join(n);
            if p.is_dir() {
                found.push(p.to_string_lossy().into_owned());
            }
        }
    }
    found
}
