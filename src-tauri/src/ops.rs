//! Thumbnails, similarity grouping, trashing and folder organisation.

use crate::photos;
use chrono::{Local, TimeZone};
use rayon::prelude::*;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    ffi::OsStr,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::UNIX_EPOCH,
};


/// Dated sub-folder for a screenshot taken at `secs`, per the folder layout setting:
/// "month" → 2026-05, "year_month" → 2026/05, "year" → 2026.
pub fn dated_dir(root: &Path, secs: i64, layout: &str) -> PathBuf {
    let Some(dt) = Local.timestamp_opt(secs, 0).single() else { return root.join("unknown") };
    match layout {
        "year_month" => root.join(dt.format("%Y").to_string()).join(dt.format("%m").to_string()),
        "year" => root.join(dt.format("%Y").to_string()),
        _ => root.join(dt.format("%Y-%m").to_string()),
    }
}
const THUMB_MAX_PX: &str = "480";

pub fn thumbs_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("clear-screenshot")
        .join("thumbs")
}

fn cache_key(path: &Path, meta: &fs::Metadata) -> String {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    meta.len().hash(&mut h);
    if let Ok(d) = meta.modified().map(|m| m.duration_since(UNIX_EPOCH)) {
        if let Ok(d) = d {
            d.as_nanos().hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

#[derive(Serialize)]
pub struct Thumb {
    pub path: String,
    pub thumb: Option<String>,
    pub hash: Option<String>,
    /// Vision feature print of the thumbnail (for "similar"); not sent to the UI.
    #[serde(skip)]
    pub fp: Option<Vec<f32>>,
}

/// Feature print of a thumbnail, cached next to it as raw little-endian f32s.
fn cached_fp(thumb: &Path, fp_file: &Path) -> Option<Vec<f32>> {
    if let Ok(b) = fs::read(fp_file) {
        if !b.is_empty() && b.len() % 4 == 0 {
            return Some(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
        }
    }
    let v = crate::vision::feature_print(thumb)?;
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    let _ = fs::write(fp_file, bytes);
    Some(v)
}

fn str_key(s: &str) -> String {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn sips_jpeg(src: &Path, max_px: &str, out: &Path) -> bool {
    Command::new("/usr/bin/sips")
        .args(["-Z", max_px, "-s", "format", "jpeg", "-s", "formatOptions", "72"])
        .arg(src)
        .arg("--out")
        .arg(out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build (or reuse) a small JPEG thumbnail, plus a perceptual hash of it.
/// Files go through `sips`; Photos assets are rendered by PhotoKit.
pub fn thumb(path: &str) -> Thumb {
    let none = || Thumb { path: path.to_string(), thumb: None, hash: None, fp: None };
    let is_photo = photos::is_photo(path);
    let p = Path::new(path);
    let dir = thumbs_dir();
    if fs::create_dir_all(&dir).is_err() {
        return none();
    }
    let key = if is_photo {
        str_key(path)
    } else {
        let Ok(meta) = fs::metadata(p) else { return none() };
        cache_key(p, &meta)
    };
    let out = dir.join(format!("{key}.jpg"));
    let hash_file = dir.join(format!("{key}.hash"));

    if !out.exists() {
        let tmp = dir.join(format!("{key}.tmp.jpg"));
        let ok = if is_photo {
            photos::render(path, THUMB_MAX_PX.parse().unwrap_or(480.0), &tmp)
        } else {
            sips_jpeg(p, THUMB_MAX_PX, &tmp)
        };
        if !ok || fs::rename(&tmp, &out).is_err() {
            let _ = fs::remove_file(&tmp);
            return none();
        }
    }

    let hash = fs::read_to_string(&hash_file)
        .ok()
        .filter(|s| s.len() == 64)
        .or_else(|| {
            let h = dhash(&out)?;
            let _ = fs::write(&hash_file, &h);
            Some(h)
        });

    Thumb { path: path.to_string(), thumb: Some(out.to_string_lossy().into_owned()), hash, fp: None }
}

/// 256-bit difference hash: compare horizontally adjacent pixels of a 17x16 grayscale image.
fn dhash(img_path: &Path) -> Option<String> {
    let img = image::open(img_path)
        .ok()?
        .grayscale()
        .resize_exact(17, 16, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut words = [0u64; 4];
    let mut bit = 0usize;
    for y in 0..16 {
        for x in 0..16 {
            if img.get_pixel(x, y)[0] < img.get_pixel(x + 1, y)[0] {
                words[bit / 64] |= 1 << (bit % 64);
            }
            bit += 1;
        }
    }
    Some(words.iter().map(|w| format!("{w:016x}")).collect())
}

fn parse_hash(s: &str) -> Option<[u64; 4]> {
    if s.len() != 64 {
        return None;
    }
    let mut w = [0u64; 4];
    for (i, slot) in w.iter_mut().enumerate() {
        *slot = u64::from_str_radix(&s[i * 16..i * 16 + 16], 16).ok()?;
    }
    Some(w)
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Group visually near-identical screenshots. Order inside each group follows `paths`.
/// Feature-print distance cutoffs per similarity setting: (any time apart, taken close together).
/// Tuned on real screenshots: under ~0.3 is the same screen; 0.3–0.5 is often the same screen or
/// flow when captured within minutes, but just a similar-looking design otherwise.
pub fn fp_cutoffs(level: &str) -> (f32, f32) {
    match level {
        "strict" => (0.24, 0.38),
        "loose" => (0.38, 0.58),
        _ => (0.30, 0.48),
    }
}
/// Screenshots taken within this many seconds of each other count as "close together".
const CLOSE_IN_TIME_SECS: i64 = 180;
/// dHash fallback cutoffs (out of 256 bits) when a feature print is missing.
fn dhash_cutoff(level: &str) -> u32 {
    match level {
        "strict" => 6,
        "loose" => 20,
        _ => 12,
    }
}

/// Group visually similar screenshots with Vision feature prints (an on-device image embedding),
/// using the capture time as context; falls back to dHash for images without a feature print.
/// `times[i]` is the capture time of `paths[i]`. Order inside each group follows `paths`.
pub fn similar_groups(paths: &[String], times: &[i64], level: &str) -> Vec<Vec<String>> {
    let thumbs: Vec<Thumb> = paths
        .par_iter()
        .map(|p| {
            let mut t = thumb(p);
            if let Some(tp) = &t.thumb {
                let tp = Path::new(tp);
                t.fp = cached_fp(tp, &tp.with_extension("fp"));
            }
            t
        })
        .collect();
    let (far, near) = fp_cutoffs(level);
    let dh = dhash_cutoff(level);
    let hashes: Vec<Option<[u64; 4]>> = thumbs.iter().map(|t| t.hash.as_deref().and_then(parse_hash)).collect();

    let n = thumbs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            let close = match (times.get(i), times.get(j)) {
                (Some(a), Some(b)) => (a - b).abs() <= CLOSE_IN_TIME_SECS,
                _ => false,
            };
            let similar = match (&thumbs[i].fp, &thumbs[j].fp) {
                (Some(a), Some(b)) if a.len() == b.len() => {
                    crate::vision::fp_distance(a, b) <= if close { near } else { far }
                }
                _ => match (hashes[i], hashes[j]) {
                    (Some(a), Some(b)) => (0..4).map(|k| (a[k] ^ b[k]).count_ones()).sum::<u32>() <= dh,
                    _ => false,
                },
            };
            if similar {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(thumbs[i].path.clone());
    }
    let mut out: Vec<Vec<String>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()));
    out
}

/// Local file to show in the full-size preview: the file itself, or a large render of a Photos asset.
pub fn preview_path(key: &str) -> Result<String, String> {
    if !photos::is_photo(key) {
        return Ok(key.to_string());
    }
    let dir = thumbs_dir().with_file_name("preview");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{}.jpg", str_key(key)));
    if out.exists() || photos::render(key, 2400.0, &out) {
        Ok(out.to_string_lossy().into_owned())
    } else {
        Err("E:preview_failed".into())
    }
}

/// Delete: files go to the macOS Trash (recoverable from Finder), Photos assets to
/// "Recently Deleted" (macOS asks for confirmation itself). Returns keys actually removed.
pub fn trash(paths: &[String]) -> Result<Vec<String>, String> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let (photo_keys, files): (Vec<String>, Vec<String>) =
        paths.iter().cloned().partition(|p| photos::is_photo(p));
    let mut removed = Vec::new();
    if !files.is_empty() {
        let mut ctx = trash::TrashContext::default();
        // NSFileManager avoids the Finder automation permission prompt.
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete_all(&files).map_err(|e| e.to_string())?;
        removed.extend(files);
    }
    if !photo_keys.is_empty() {
        match photos::delete(&photo_keys) {
            Ok(_) => removed.extend(photo_keys),
            // Files already went to the Trash; report the Photos failure but keep that progress.
            Err(e) if !removed.is_empty() => eprintln!("photos delete failed: {e}"),
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

#[derive(Serialize)]
pub struct Moved {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Debug)]
pub struct Failed {
    /// Item key the error is about ("" for errors about the whole batch).
    pub path: String,
    pub error: String,
}

#[derive(Serialize)]
pub struct OrganizeResult {
    pub moved: Vec<Moved>,
    pub failed: Vec<Failed>,
}

pub fn unique_dest(dir: &Path, name: &OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let name = Path::new(name);
    let stem = name.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

fn organize_one(src: &Path, root: &Path, layout: &str) -> std::io::Result<Option<PathBuf>> {
    let meta = fs::metadata(src)?;
    let t = meta.created().or_else(|_| meta.modified())?;
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let dir = dated_dir(root, secs, layout);
    // APFS is normalization-insensitive, so compare paths in NFC: a byte-level mismatch
    // would otherwise "move" a file onto itself and rename it to "name (1)".
    let norm = |p: &Path| -> String { p.to_string_lossy().nfc().collect() };
    if src.parent().map(norm) == Some(norm(&dir)) {
        return Ok(None);
    }
    fs::create_dir_all(&dir)?;
    let name = src.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let dest = unique_dest(&dir, name);
    if fs::rename(src, &dest).is_err() {
        fs::copy(src, &dest)?;
        fs::remove_file(src)?;
    }
    Ok(Some(dest))
}

/// Move each file into `<root>/<YYYY-MM>/`.
/// Files: move into `<root>/<YYYY-MM>/`.
/// Photos assets: export into the same folders, then delete them from Photos in one batch
/// (one macOS confirmation). If that deletion is cancelled or fails, the exported copies are
/// removed again so nothing ends up duplicated.
pub fn organize(paths: &[String], root: &str, layout: &str) -> OrganizeResult {
    let root = Path::new(root);
    let mut moved = Vec::new();
    let mut failed = Vec::new();
    let (photo_keys, files): (Vec<&String>, Vec<&String>) = paths.iter().partition(|p| photos::is_photo(p));

    for p in files {
        match organize_one(Path::new(p), root, layout) {
            Ok(Some(dest)) => moved.push(Moved { from: p.clone(), to: dest.to_string_lossy().into_owned() }),
            Ok(None) => {}
            Err(e) => failed.push(Failed { path: p.clone(), error: e.to_string() }),
        }
    }

    let mut exported: Vec<(String, PathBuf)> = Vec::new();
    for k in photo_keys {
        match photos::export(k, root, layout) {
            Ok(dest) => exported.push((k.clone(), dest)),
            Err(e) => failed.push(Failed { path: k.clone(), error: e }),
        }
    }
    if !exported.is_empty() {
        let keys: Vec<String> = exported.iter().map(|(k, _)| k.clone()).collect();
        match photos::delete(&keys) {
            Ok(_) => moved.extend(exported.into_iter().map(|(k, p)| Moved {
                from: k,
                to: p.to_string_lossy().into_owned(),
            })),
            Err(e) => {
                for (_, p) in &exported {
                    let _ = fs::remove_file(p);
                    if let Some(dir) = p.parent() {
                        let _ = fs::remove_dir(dir); // only succeeds if now empty
                    }
                }
                failed.push(Failed {
                    path: String::new(),
                    error: format!("E:photos_rollback|{}|{e}", exported.len()),
                });
            }
        }
    }
    OrganizeResult { moved, failed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clear-screenshot-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn month_of(p: &Path) -> String {
        let t = fs::metadata(p).unwrap().created().unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        Local.timestamp_opt(secs, 0).unwrap().format("%Y-%m").to_string()
    }

    #[test]
    fn moves_all_files_into_month_folder_and_is_idempotent() {
        let root = tmp_root("move");
        let names = ["스크린샷 a.png", "스크린샷 b.png", "Screenshot c.png"];
        let paths: Vec<String> = names
            .iter()
            .map(|n| {
                let p = root.join(n);
                fs::write(&p, b"x").unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        let month = month_of(Path::new(&paths[0]));

        let r = organize(&paths, root.to_str().unwrap(), "month");
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.moved.len(), 3);
        for n in names {
            assert!(root.join(&month).join(n).exists(), "{n} not moved");
            assert!(!root.join(n).exists());
        }

        // Running again on the moved files must do nothing (no "(1)" copies).
        let moved: Vec<String> = r.moved.iter().map(|m| m.to.clone()).collect();
        let again = organize(&moved, root.to_str().unwrap(), "month");
        assert!(again.moved.is_empty() && again.failed.is_empty());
        assert_eq!(fs::read_dir(root.join(&month)).unwrap().count(), 3);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nfd_vs_nfc_root_is_treated_as_same_folder() {
        let base = tmp_root("nfd");
        let nfd_root = base.join("스크린샷".nfd().collect::<String>());
        let nfc_root = base.join("스크린샷".nfc().collect::<String>());
        fs::create_dir_all(&nfd_root).unwrap();
        let f = nfd_root.join("a.png");
        fs::write(&f, b"x").unwrap();
        let month = month_of(&f);
        let r = organize(&[f.to_string_lossy().into_owned()], nfd_root.to_str().unwrap(), "month");
        assert_eq!(r.moved.len(), 1);
        // Same file referenced through the NFC spelling of the root: already organized.
        let inside = nfd_root.join(&month).join("a.png");
        let again = organize(&[inside.to_string_lossy().into_owned()], nfc_root.to_str().unwrap(), "month");
        assert!(again.moved.is_empty(), "renamed onto itself: {:?}", again.moved.iter().map(|m| &m.to).collect::<Vec<_>>());
        assert!(inside.exists());
        fs::remove_dir_all(&base).unwrap();
    }
}

#[cfg(test)]
mod export_helpers {
    use crate::photos::{backdate, korean_stamp};
    use chrono::{Local, TimeZone};
    use std::{fs, time::UNIX_EPOCH};

    #[test]
    fn stamp_matches_macos_naming() {
        let dt = Local.with_ymd_and_hms(2026, 6, 2, 18, 10, 7).unwrap();
        assert_eq!(korean_stamp(dt), "2026-06-02 오후 6.10.07");
        let dt = Local.with_ymd_and_hms(2026, 5, 11, 0, 3, 9).unwrap();
        assert_eq!(korean_stamp(dt), "2026-05-11 오전 12.03.09");
        let dt = Local.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        assert_eq!(korean_stamp(dt), "2026-05-11 오후 12.00.00");
    }

    #[test]
    fn backdate_moves_creation_time_too() {
        let p = std::env::temp_dir().join(format!("cs-backdate-{}.png", std::process::id()));
        fs::write(&p, b"x").unwrap();
        let target = Local.with_ymd_and_hms(2025, 3, 4, 10, 0, 0).unwrap().timestamp();
        backdate(&p, target as f64);
        let m = fs::metadata(&p).unwrap();
        let secs = |t: std::time::SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        assert_eq!(secs(m.modified().unwrap()), target);
        assert_eq!(secs(m.created().unwrap()), target, "birth time not moved back");
        fs::remove_file(&p).unwrap();
    }
}

// ---------- OCR analysis ----------

#[derive(Serialize)]
pub struct Analysis {
    pub path: String,
    pub ok: bool,
    pub text: String,
    /// What is in the picture (Vision identifiers + confidence), most confident first.
    pub objects: Vec<crate::vision::Obj>,
    /// A screenshot of Clear Screenshot itself (its text is our UI).
    pub own_app: bool,
    /// Apple on-device model's description of the picture, if available.
    pub ai: Option<crate::ai::AiDesc>,
    pub tags: Vec<crate::classify::Tag>,
}

fn ocr_dir() -> PathBuf {
    thumbs_dir().with_file_name("ocr-v1")
}

/// File OCR is keyed by size + mtime (not path) so moving/organizing a file keeps its cache.
fn ocr_key(path: &str) -> Option<String> {
    if photos::is_photo(path) {
        return Some(str_key(path));
    }
    let meta = fs::metadata(path).ok()?;
    let mut h = DefaultHasher::new();
    meta.len().hash(&mut h);
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos().hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

fn ocr_base(path: &str) -> Option<crate::vision::Ocr> {
    let dir = ocr_dir();
    fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("{}.json", ocr_key(path)?));
    // Photos assets are OCR'd from a large render; files from the original.
    let source = || -> Option<PathBuf> {
        Some(if photos::is_photo(path) { PathBuf::from(preview_path(path).ok()?) } else { PathBuf::from(path) })
    };
    if let Some(mut o) = fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str::<crate::vision::Ocr>(&s).ok()) {
        if o.v >= crate::vision::VERSION {
            return Some(o);
        }
        // Older cache entry: keep its text and run only the analyses added since it was written.
        let Some(src) = source() else { return Some(o) };
        let mut ok = true;
        if o.v < 2 {
            match crate::vision::detect_codes(&src) {
                Ok((qr, barcodes)) => { o.qr = qr; o.barcodes = barcodes; }
                Err(_) => ok = false,
            }
        }
        if o.v < 3 {
            match crate::vision::detect_objects(&src) {
                Ok(objects) => o.objects = objects,
                Err(_) => ok = false,
            }
        }
        if ok {
            o.v = crate::vision::VERSION;
            let _ = fs::write(&file, serde_json::to_string(&o).ok()?);
        }
        return Some(o);
    }
    let o = crate::vision::recognize(&source()?).ok()?;
    let _ = fs::write(&file, serde_json::to_string(&o).ok()?);
    Some(o)
}

/// True if analyzing `path` will do real work (no up-to-date stored result yet).
/// Lets the UI show the first-run progress window only when there is something to wait for,
/// and makes an interrupted run resume where it stopped (finished screenshots are stored).
pub fn needs_analysis(path: &str, want_ai: bool) -> bool {
    let Some(key) = ocr_key(path) else { return true };
    let Some(o) = fs::read_to_string(ocr_dir().join(format!("{key}.json")))
        .ok()
        .and_then(|s| serde_json::from_str::<crate::vision::Ocr>(&s).ok())
    else {
        return true;
    };
    o.v < crate::vision::VERSION || (want_ai && o.ai_v < crate::ai::AI_VERSION && crate::ai::available())
}

/// The image to analyze: files as they are, Photos assets through a large render.
fn analysis_source(path: &str) -> Option<PathBuf> {
    Some(if photos::is_photo(path) { PathBuf::from(preview_path(path).ok()?) } else { PathBuf::from(path) })
}

/// Cached OCR/barcode/object analysis, plus (when `want_ai`) the on-device model's description,
/// which is asked once per screenshot and stored with the rest.
fn ocr_cached(path: &str, want_ai: bool) -> Option<crate::vision::Ocr> {
    let mut o = ocr_base(path)?;
    if want_ai && o.ai_v < crate::ai::AI_VERSION && crate::ai::available() {
        o.ai = crate::ai::describe(&analysis_source(path)?);
        o.ai_tried = true;
        o.ai_v = crate::ai::AI_VERSION;
        let file = ocr_dir().join(format!("{}.json", ocr_key(path)?));
        let _ = fs::write(&file, serde_json::to_string(&o).ok()?);
    }
    Some(o)
}

pub fn analyze(path: &str, want_ai: bool) -> Analysis {
    match ocr_cached(path, want_ai) {
        Some(o) => {
            // A screenshot of this very app: its text is our own UI, not content — keep it
            // searchable, but don't derive tags (and the UI won't derive categories) from it.
            let own = crate::classify::is_own_app_capture(&o.text);
            Analysis {
                path: path.to_string(),
                ok: true,
                tags: if own { vec![] } else { crate::classify::classify(&o.text, &o.qr, &o.barcodes) },
                own_app: own,
                text: o.text,
                objects: o.objects,
                ai: o.ai,
            }
        }
        None => Analysis { path: path.to_string(), ok: false, own_app: false, text: String::new(), objects: vec![], tags: vec![], ai: None },
    }
}

// ---------- cache management ----------

fn dir_size(p: &Path) -> u64 {
    fs::read_dir(p)
        .map(|it| {
            it.flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => dir_size(&e.path()),
                    _ => e.metadata().map(|m| m.len()).unwrap_or(0),
                })
                .sum()
        })
        .unwrap_or(0)
}

#[derive(Serialize)]
pub struct CacheInfo {
    /// thumbnails + large previews (cheap to rebuild)
    pub images: u64,
    /// text recognition results (slow to rebuild)
    pub ocr: u64,
}

pub fn cache_info() -> CacheInfo {
    let thumbs = thumbs_dir();
    CacheInfo {
        images: dir_size(&thumbs) + dir_size(&thumbs.with_file_name("preview")),
        ocr: dir_size(&ocr_dir()),
    }
}

/// `what`: "images" (thumbnails/previews) or "ocr" (text recognition results).
pub fn clear_cache(what: &str) -> Result<(), String> {
    let thumbs = thumbs_dir();
    let dirs = match what {
        "images" => vec![thumbs.clone(), thumbs.with_file_name("preview")],
        "ocr" => vec![ocr_dir()],
        _ => return Err("unknown cache".into()),
    };
    for d in dirs {
        if d.exists() {
            fs::remove_dir_all(&d).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod layout_tests {
    use super::dated_dir;
    use chrono::{Local, TimeZone};
    use std::path::Path;

    #[test]
    fn folder_layouts_match_the_ui() {
        let t = Local.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap().timestamp();
        let root = Path::new("/r");
        assert_eq!(dated_dir(root, t, "month"), Path::new("/r/2026-05"));
        assert_eq!(dated_dir(root, t, "year_month"), Path::new("/r/2026/05"));
        assert_eq!(dated_dir(root, t, "year"), Path::new("/r/2026"));
        assert_eq!(dated_dir(root, t, "anything-else"), Path::new("/r/2026-05"));
    }
}

#[cfg(test)]
mod fp_tuning {
    /// FP_DIR=folder cargo test fp_bands -- --ignored --nocapture
    /// For tuning the similarity cutoff: shows the stored AI one-line descriptions of image pairs
    /// at various feature-print distances (no images are shown).
    #[test]
    #[ignore]
    fn fp_bands() {
        let dir = std::env::var("FP_DIR").expect("set FP_DIR");
        let files: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p.ends_with(".png")).collect();
        let items: Vec<(String, Vec<f32>, String)> = files.iter().filter_map(|p| {
            let fp = crate::vision::feature_print(std::path::Path::new(p))?;
            let o = super::ocr_base(p)?;
            Some((p.rsplit('/').next().unwrap().to_string(), fp, o.ai.map(|a| a.summary).unwrap_or_default()))
        }).collect();
        let mut pairs = vec![];
        for i in 0..items.len() { for j in i + 1..items.len() {
            pairs.push((crate::vision::fp_distance(&items[i].1, &items[j].1), i, j));
        } }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (lo, hi) in [(0.0f32, 0.3f32), (0.3, 0.4), (0.4, 0.5), (0.5, 0.6)] {
            println!("== {lo}–{hi}: {} pairs", pairs.iter().filter(|p| p.0 >= lo && p.0 < hi).count());
            for (d, i, j) in pairs.iter().filter(|p| p.0 >= lo && p.0 < hi).take(4) {
                println!("  {d:.2}  [{}] {}\n        [{}] {}", items[*i].0, &items[*i].2.chars().take(70).collect::<String>(), items[*j].0, &items[*j].2.chars().take(70).collect::<String>());
            }
        }
    }
}

#[cfg(test)]
mod similar_tests {
    use super::fp_cutoffs;

    #[test]
    fn cutoffs_are_ordered() {
        for level in ["strict", "normal", "loose"] {
            let (far, near) = fp_cutoffs(level);
            assert!(far < near, "taken close together must be looser ({level})");
        }
        assert!(fp_cutoffs("strict").0 < fp_cutoffs("normal").0 && fp_cutoffs("normal").0 < fp_cutoffs("loose").0);
    }
}

#[cfg(test)]
mod similar_compare {
    /// FP_DIR=folder cargo test similar_compare_run -- --ignored --nocapture
    #[test]
    #[ignore]
    fn similar_compare_run() {
        let dir = std::env::var("FP_DIR").expect("set FP_DIR");
        let mut files: Vec<String> = std::fs::read_dir(dir).unwrap().flatten()
            .map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.ends_with(".png")).collect();
        files.sort();
        let times: Vec<i64> = files.iter().map(|p| std::fs::metadata(p).unwrap().modified().unwrap()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64).collect();
        // old method: dHash only (pretend no feature prints by giving no times and comparing hashes)
        let thumbs: Vec<super::Thumb> = files.iter().map(|p| super::thumb(p)).collect();
        let hs: Vec<[u64; 4]> = thumbs.iter().filter_map(|t| super::parse_hash(t.hash.as_deref()?)).collect();
        let mut old_pairs = 0;
        for i in 0..hs.len() { for j in i + 1..hs.len() {
            if (0..4).map(|k| (hs[i][k] ^ hs[j][k]).count_ones()).sum::<u32>() <= 12 { old_pairs += 1; }
        } }
        println!("images={} old dHash similar pairs={}", files.len(), old_pairs);
        for level in ["strict", "normal", "loose"] {
            let g = super::similar_groups(&files, &times, level);
            println!("{level}: groups={} screenshots in groups={} sizes={:?}", g.len(), g.iter().map(|x| x.len()).sum::<usize>(),
                g.iter().map(|x| x.len()).collect::<Vec<_>>());
        }
        let g = super::similar_groups(&files, &times, "normal");
        for grp in g.iter().take(4) {
            let names: Vec<&str> = grp.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
            let sums: Vec<String> = grp.iter().map(|p| super::ocr_base(p).and_then(|o| o.ai).map(|a| a.summary.chars().take(45).collect()).unwrap_or_default()).collect();
            println!("  group: {:?}\n         {:?}", names, sums);
        }
    }
}
