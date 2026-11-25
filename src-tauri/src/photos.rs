//! Photos library (PhotoKit) access: list the Screenshots smart album, render
//! thumbnails/previews, and delete assets (they go to "Recently Deleted").
//!
//! Photos items are addressed in the UI as `photos://<localIdentifier>` so they can share
//! the same key space as file paths.

use crate::scan::Shot;
use block2::RcBlock;
use chrono::{DateTime, Local, TimeZone, Timelike};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_core_foundation::CGSize;
use objc2_image_io::CGImagePropertyOrientation;
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSNumber, NSObjectNSKeyValueCoding, NSString, NSURL,
};
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetChangeRequest, PHAssetCollection, PHAssetCollectionSubtype,
    PHAssetCollectionType, PHAssetResource, PHAssetResourceManager, PHAssetResourceRequestOptions,
    PHAssetResourceType, PHAuthorizationStatus, PHFetchResult, PHImageContentMode, PHImageManager,
    PHImageRequestOptions, PHImageRequestOptionsDeliveryMode, PHPhotoLibrary,
};
use serde::Serialize;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

pub const PREFIX: &str = "photos://";

pub fn is_photo(key: &str) -> bool {
    key.starts_with(PREFIX)
}

fn id_of(key: &str) -> &str {
    &key[PREFIX.len()..]
}

#[derive(Serialize)]
pub struct PhotosScan {
    /// "authorized" | "limited" | "denied"
    pub status: String,
    pub items: Vec<Shot>,
}

fn authorize() -> PHAuthorizationStatus {
    unsafe {
        let st = PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite);
        if st != PHAuthorizationStatus::NotDetermined {
            return st;
        }
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |s: PHAuthorizationStatus| {
            let _ = tx.send(s);
        });
        PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(PHAccessLevel::ReadWrite, &block);
        rx.recv().unwrap_or(PHAuthorizationStatus::Denied)
    }
}

unsafe fn shot_of(asset: &PHAsset) -> Shot {
    let id = asset.localIdentifier().to_string();
    let created = asset
        .creationDate()
        .map(|d| d.timeIntervalSince1970() as i64)
        .unwrap_or(0);
    let resources = PHAssetResource::assetResourcesForAsset(asset);
    let (name, size) = match resources.firstObject() {
        Some(r) => {
            let size = r
                .valueForKey(&NSString::from_str("fileSize"))
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .map(|n| n.longLongValue().max(0) as u64)
                .unwrap_or(0);
            (r.originalFilename().to_string(), size)
        }
        None => ("Photo".to_string(), 0),
    };
    let (width, height) = (asset.pixelWidth() as u32, asset.pixelHeight() as u32);
    Shot {
        path: format!("{PREFIX}{id}"),
        name,
        size,
        created,
        width,
        height,
        // Photos assets never carry macOS's capture tag; identify by screen resolution.
        device: crate::device::detect(width, height, false),
    }
}

/// All assets in the Photos "Screenshots" smart album. Prompts for access on first use.
pub fn scan() -> PhotosScan {
    let status = match authorize() {
        PHAuthorizationStatus::Authorized => "authorized",
        PHAuthorizationStatus::Limited => "limited",
        _ => "denied",
    };
    let mut items = Vec::new();
    if status != "denied" {
        unsafe {
            let colls = PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(
                PHAssetCollectionType::SmartAlbum,
                PHAssetCollectionSubtype::SmartAlbumScreenshots,
                None,
            );
            for ci in 0..colls.count() {
                let assets = PHAsset::fetchAssetsInAssetCollection_options(&colls.objectAtIndex(ci), None);
                for i in 0..assets.count() {
                    items.push(shot_of(&assets.objectAtIndex(i)));
                }
            }
        }
    }
    PhotosScan { status: status.to_string(), items }
}

unsafe fn fetch(keys: &[String]) -> Retained<PHFetchResult<PHAsset>> {
    let ids: Vec<Retained<NSString>> = keys.iter().map(|k| NSString::from_str(id_of(k))).collect();
    PHAsset::fetchAssetsWithLocalIdentifiers_options(&NSArray::from_retained_slice(&ids), None)
}

fn to_jpeg(img: &NSImage) -> Option<Vec<u8>> {
    let tiff = img.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let props = NSDictionary::new();
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &props) }?;
    Some(data.to_vec())
}

/// Render the asset to a JPEG no larger than `max_px` on its long side.
pub fn render(key: &str, max_px: f64, out: &Path) -> bool {
    let jpeg: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    unsafe {
        let found = fetch(&[key.to_string()]);
        if found.count() == 0 {
            return false;
        }
        let asset = found.objectAtIndex(0);
        let opts = PHImageRequestOptions::new();
        opts.setSynchronous(true);
        opts.setDeliveryMode(PHImageRequestOptionsDeliveryMode::HighQualityFormat);
        opts.setNetworkAccessAllowed(true);
        let slot = jpeg.clone();
        let handler = RcBlock::new(move |img: *mut NSImage, _info: *mut NSDictionary| {
            if let Some(img) = img.as_ref() {
                if let Some(bytes) = to_jpeg(img) {
                    *slot.lock().unwrap() = Some(bytes);
                }
            }
        });
        PHImageManager::defaultManager().requestImageForAsset_targetSize_contentMode_options_resultHandler(
            &asset,
            CGSize { width: max_px, height: max_px },
            PHImageContentMode::AspectFit,
            Some(&opts),
            &handler,
        );
    }
    let bytes = jpeg.lock().unwrap().take();
    match bytes {
        Some(b) => fs::write(out, b).is_ok(),
        None => false,
    }
}

/// "2026-06-02 오후 6.10.07" — the stamp macOS uses in Korean screenshot file names.
pub fn korean_stamp(dt: DateTime<Local>) -> String {
    let h = dt.hour();
    let (ampm, h12) = if h < 12 { ("오전", if h == 0 { 12 } else { h }) } else { ("오후", if h == 12 { 12 } else { h - 12 }) };
    format!("{} {ampm} {h12}.{:02}.{:02}", dt.format("%Y-%m-%d"), dt.minute(), dt.second())
}

/// Set a file's modification time; on APFS this also moves the creation (birth) time back
/// when the new time is earlier, so the file sorts under the date the screenshot was taken.
pub fn backdate(path: &Path, secs: f64) {
    let t = UNIX_EPOCH + Duration::from_secs_f64(secs.max(0.0));
    if let Ok(f) = fs::File::options().write(true).open(path) {
        let _ = f.set_modified(t);
    }
}

fn ext_of(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into())
}

fn uti_ext(uti: &str) -> &'static str {
    match uti {
        "public.png" => "png",
        "public.jpeg" => "jpg",
        "public.heic" | "public.heif" => "heic",
        "public.tiff" => "tiff",
        "com.compuserve.gif" => "gif",
        _ => "png",
    }
}

fn nonempty(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

/// Stream one asset resource to `dest` with PHAssetResourceManager.
unsafe fn write_resource(res: &PHAssetResource, dest: &Path) -> Result<(), String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&dest.to_string_lossy()));
    let opts = PHAssetResourceRequestOptions::new();
    opts.setNetworkAccessAllowed(true); // originals may live only in iCloud
    let (tx, rx) = mpsc::channel();
    let done = RcBlock::new(move |err: *mut NSError| {
        let _ = tx.send(err.as_ref().map(|e| e.localizedDescription().to_string()));
    });
    PHAssetResourceManager::defaultManager()
        .writeDataForAssetResource_toFile_options_completionHandler(res, &url, Some(&opts), &done);
    let result = match rx.recv_timeout(Duration::from_secs(180)) {
        Ok(None) if nonempty(dest) => Ok(()),
        Ok(None) => Err("E:export_empty".into()),
        Ok(Some(e)) => Err(e),
        Err(_) => Err("E:export_timeout".into()),
    };
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}

/// Ask PHImageManager for the encoded image bytes (current, i.e. edited, version).
unsafe fn image_data(asset: &PHAsset) -> Option<(Vec<u8>, String)> {
    let slot: Arc<Mutex<Option<(Vec<u8>, String)>>> = Arc::new(Mutex::new(None));
    let opts = PHImageRequestOptions::new();
    opts.setSynchronous(true);
    opts.setNetworkAccessAllowed(true);
    opts.setDeliveryMode(PHImageRequestOptionsDeliveryMode::HighQualityFormat);
    let s = slot.clone();
    let handler = RcBlock::new(
        move |data: *mut NSData, uti: *mut NSString, _o: CGImagePropertyOrientation, _info: *mut NSDictionary| {
            if let Some(d) = data.as_ref() {
                let uti = uti.as_ref().map(|u| u.to_string()).unwrap_or_default();
                *s.lock().unwrap() = Some((d.to_vec(), uti));
            }
        },
    );
    PHImageManager::defaultManager().requestImageDataAndOrientationForAsset_options_resultHandler(
        asset,
        Some(&opts),
        &handler,
    );
    let out = slot.lock().unwrap().take();
    out.filter(|(b, _)| !b.is_empty())
}

/// Write the asset's displayed version to `<root>/<YYYY-MM>/스크린샷 <stamp>.<ext>`.
/// Tries, in order: the asset's resource files (edited, original, alternate, pre-edit base),
/// the encoded image data from PHImageManager, and finally a full-size JPEG render.
/// Does not delete anything.
pub fn export(key: &str, root: &Path, layout: &str) -> Result<PathBuf, String> {
    unsafe {
        let found = fetch(&[key.to_string()]);
        if found.count() == 0 {
            return Err("E:photo_not_found".into());
        }
        let asset = found.objectAtIndex(0);
        let created = asset.creationDate().map(|d| d.timeIntervalSince1970()).unwrap_or(0.0);
        let dt = Local.timestamp_opt(created as i64, 0).single().unwrap_or_else(Local::now);
        let dir = crate::ops::dated_dir(root, created as i64, layout);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stem = format!("스크린샷 {}", korean_stamp(dt));
        let dest_for = |ext: &str| crate::ops::unique_dest(&dir, OsStr::new(&format!("{stem}.{ext}")));
        let finish = |dest: PathBuf| {
            backdate(&dest, created);
            Ok(dest)
        };

        let mut last_err = String::from("E:export_failed");
        let resources = PHAssetResource::assetResourcesForAsset(&asset);
        for t in [
            PHAssetResourceType::FullSizePhoto,
            PHAssetResourceType::Photo,
            PHAssetResourceType::AlternatePhoto,
            PHAssetResourceType::AdjustmentBasePhoto,
        ] {
            let Some(res) = (0..resources.count()).map(|i| resources.objectAtIndex(i)).find(|r| r.r#type() == t) else {
                continue;
            };
            let dest = dest_for(&ext_of(&res.originalFilename().to_string()));
            match write_resource(&res, &dest) {
                Ok(()) => return finish(dest),
                Err(e) => last_err = e,
            }
        }

        if let Some((bytes, uti)) = image_data(&asset) {
            let dest = dest_for(uti_ext(&uti));
            if fs::write(&dest, bytes).is_ok() && nonempty(&dest) {
                return finish(dest);
            }
            let _ = fs::remove_file(&dest);
        }

        let dest = dest_for("jpg");
        if render(key, 16384.0, &dest) && nonempty(&dest) {
            return finish(dest);
        }
        let _ = fs::remove_file(&dest);
        let _ = fs::remove_dir(&dir); // only if we left it empty
        Err(last_err)
    }
}

/// Delete assets. macOS shows its own confirmation; they land in "Recently Deleted".
pub fn delete(keys: &[String]) -> Result<usize, String> {
    if keys.is_empty() {
        return Ok(0);
    }
    unsafe {
        let assets = fetch(keys);
        let n = assets.count();
        if n == 0 {
            return Ok(0);
        }
        let captured = assets.clone();
        let change = RcBlock::new(move || {
            PHAssetChangeRequest::deleteAssets(ProtocolObject::from_ref(&*captured));
        });
        PHPhotoLibrary::sharedPhotoLibrary()
            .performChangesAndWait_error(&*change as *const _ as *mut _)
            .map_err(|e| e.localizedDescription().to_string())?;
        Ok(n)
    }
}
