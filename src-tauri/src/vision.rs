//! On-device text recognition (Korean + English), barcode/QR detection and object classification
//! ("what is in the picture": coffee, dog, food …) with Apple's Vision framework.

use objc2::{rc::{autoreleasepool, Retained}, AllocAnyThread};
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
use objc2_core_foundation::CGRect;
use objc2_vision::{
    VNBarcodeSymbologyQR, VNClassifyImageRequest, VNDetectBarcodesRequest, VNElementType,
    VNGenerateImageFeaturePrintRequest,
    VNGenerateAttentionBasedSaliencyImageRequest, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

/// Bump when cached results need new fields filled in (see `ops::ocr_cached`).
/// v2: all barcode symbologies, not only QR.
/// v3: objects in the picture (Vision image classification).
pub const VERSION: u32 = 3;

/// Keep labels at least this confident, at most this many per screenshot.
const OBJECT_MIN_CONFIDENCE: f32 = 0.2;
const OBJECT_MAX: usize = 15;
/// Re-classify the main picture region only if it covers this share of the screenshot.
const REGION_MIN_AREA: f64 = 0.04;
const REGION_MAX_AREA: f64 = 0.85;

/// One thing Vision recognized in the picture, e.g. {id: "coffee", conf: 0.83}.
/// `id` is Vision's English identifier; the UI maps search words onto it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Obj {
    pub id: String,
    pub conf: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Ocr {
    /// Recognized lines, top to bottom, joined with '\n'.
    pub text: String,
    /// Payloads of QR codes found in the image.
    pub qr: Vec<String>,
    /// Symbology names of other barcodes found (e.g. "VNBarcodeSymbologyCode128").
    #[serde(default)]
    pub barcodes: Vec<String>,
    /// What is in the picture, most confident first.
    #[serde(default)]
    pub objects: Vec<Obj>,
    /// Apple on-device model's understanding of the picture (None if unavailable/refused).
    #[serde(default)]
    pub ai: Option<crate::ai::AiDesc>,
    /// The on-device model has been asked once (so a refusal is not retried every launch).
    #[serde(default)]
    pub ai_tried: bool,
    /// `ai::AI_VERSION` the stored description was made with.
    #[serde(default)]
    pub ai_v: u32,
    #[serde(default)]
    pub v: u32,
}

fn handler_for(image: &Path) -> Retained<VNImageRequestHandler> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&image.to_string_lossy()));
    unsafe { VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new()) }
}

/// (QR payloads, other barcode symbologies) from a finished barcode request.
fn collect_codes(req: &VNDetectBarcodesRequest) -> (Vec<String>, Vec<String>) {
    let qr_name = unsafe { VNBarcodeSymbologyQR }.map(|s| s.to_string()).unwrap_or_default();
    let (mut qr, mut other) = (Vec::new(), Vec::new());
    if let Some(results) = unsafe { req.results() } {
        for obs in results.iter() {
            let sym = unsafe { obs.symbology() }.to_string();
            if sym == qr_name {
                qr.push(unsafe { obs.payloadStringValue() }.map(|s| s.to_string()).unwrap_or_default());
            } else {
                other.push(sym);
            }
        }
    }
    (qr, other)
}

/// Highest confidence per identifier, from a finished classification request.
fn collect_objects(req: &VNClassifyImageRequest, into: &mut HashMap<String, f32>) {
    if let Some(results) = unsafe { req.results() } {
        for obs in results.iter() {
            let conf = unsafe { obs.confidence() };
            if conf < OBJECT_MIN_CONFIDENCE {
                continue;
            }
            let id = unsafe { obs.identifier() }.to_string();
            let e = into.entry(id).or_insert(0.0);
            *e = e.max(conf);
        }
    }
}

/// The biggest "where people look" region (normalized), if it is a picture inside the screenshot
/// rather than the whole screen or a speck. Screenshots of feeds/chats are mostly UI chrome, so
/// classifying this region on its own finds the photo's subject much more reliably.
fn main_region(req: &VNGenerateAttentionBasedSaliencyImageRequest) -> Option<CGRect> {
    let obs = unsafe { req.results() }?.firstObject()?;
    let boxes = unsafe { obs.salientObjects() }?;
    boxes
        .iter()
        .map(|b| unsafe { b.boundingBox() })
        .filter(|r| {
            let area = r.size.width * r.size.height;
            (REGION_MIN_AREA..=REGION_MAX_AREA).contains(&area)
        })
        .max_by(|a, b| (a.size.width * a.size.height).total_cmp(&(b.size.width * b.size.height)))
}

fn finish_objects(found: HashMap<String, f32>) -> Vec<Obj> {
    let mut v: Vec<Obj> = found.into_iter().map(|(id, conf)| Obj { id, conf }).collect();
    v.sort_by(|a, b| b.conf.total_cmp(&a.conf));
    v.truncate(OBJECT_MAX);
    v
}

/// Classify the whole screenshot, then (if there is one) its main picture region, and merge.
/// `first` and `saliency` must already have been performed on `handler`.
fn objects_after(
    handler: &VNImageRequestHandler,
    first: &VNClassifyImageRequest,
    saliency: &VNGenerateAttentionBasedSaliencyImageRequest,
) -> Vec<Obj> {
    let mut found = HashMap::new();
    collect_objects(first, &mut found);
    if let Some(region) = main_region(saliency) {
        let crop = unsafe { VNClassifyImageRequest::new() };
        unsafe { crop.setRegionOfInterest(region) };
        let r: &VNRequest = &crop;
        if handler.performRequests_error(&NSArray::from_slice(&[r])).is_ok() {
            collect_objects(&crop, &mut found);
        }
    }
    finish_objects(found)
}

/// Object classification only (used to upgrade results cached before v3 without re-running OCR).
pub fn detect_objects(image: &Path) -> Result<Vec<Obj>, String> {
    autoreleasepool(|_| {
        let handler = handler_for(image);
        let cls = unsafe { VNClassifyImageRequest::new() };
        let sal = unsafe { VNGenerateAttentionBasedSaliencyImageRequest::new() };
        let (a, b): (&VNRequest, &VNRequest) = (&cls, &sal);
        handler.performRequests_error(&NSArray::from_slice(&[a, b])).map_err(|e| e.localizedDescription().to_string())?;
        Ok(objects_after(&handler, &cls, &sal))
    })
}

/// Apple's image "feature print": a learned embedding of what the picture looks like, robust to
/// scrolling, cropping and brightness changes. Two screenshots of the same screen are close.
pub fn feature_print(image: &Path) -> Option<Vec<f32>> {
    autoreleasepool(|_| {
        let handler = handler_for(image);
        let req = unsafe { VNGenerateImageFeaturePrintRequest::new() };
        let r: &VNRequest = &req;
        handler.performRequests_error(&NSArray::from_slice(&[r])).ok()?;
        let obs = unsafe { req.results() }?.firstObject()?;
        let bytes = unsafe { obs.data() }.to_vec();
        let v: Vec<f32> = match unsafe { obs.elementType() } {
            VNElementType::Float => bytes.chunks_exact(4).map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect(),
            VNElementType::Double => bytes
                .chunks_exact(8)
                .map(|c| f64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32)
                .collect(),
            _ => return None,
        };
        (!v.is_empty()).then_some(v)
    })
}

/// Euclidean distance between two feature prints (what Vision's computeDistance uses).
pub fn fp_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

/// Barcode detection only (used to upgrade results cached before v2 without re-running OCR).
pub fn detect_codes(image: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    autoreleasepool(|_| {
        let handler = handler_for(image);
        let req = unsafe { VNDetectBarcodesRequest::new() }; // no symbology filter = all supported
        let r: &VNRequest = &req;
        handler.performRequests_error(&NSArray::from_slice(&[r])).map_err(|e| e.localizedDescription().to_string())?;
        Ok(collect_codes(&req))
    })
}

pub fn recognize(image: &Path) -> Result<Ocr, String> {
    autoreleasepool(|_| {
        let handler = handler_for(image);

        let text_req = VNRecognizeTextRequest::new();
        text_req.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        text_req.setRecognitionLanguages(&NSArray::from_retained_slice(&[
            NSString::from_str("ko-KR"),
            NSString::from_str("en-US"),
        ]));
        text_req.setUsesLanguageCorrection(true);
        let code_req = unsafe { VNDetectBarcodesRequest::new() };
        let cls_req = unsafe { VNClassifyImageRequest::new() };
        let sal_req = unsafe { VNGenerateAttentionBasedSaliencyImageRequest::new() };

        let reqs: [&VNRequest; 4] = [&text_req, &code_req, &cls_req, &sal_req];
        handler
            .performRequests_error(&NSArray::from_slice(&reqs))
            .map_err(|e| e.localizedDescription().to_string())?;

        let mut lines = Vec::new();
        if let Some(results) = text_req.results() {
            for obs in results.iter() {
                if let Some(best) = obs.topCandidates(1).firstObject() {
                    lines.push(best.string().to_string());
                }
            }
        }
        let (qr, barcodes) = collect_codes(&code_req);
        let objects = objects_after(&handler, &cls_req, &sal_req);
        Ok(Ocr { text: lines.join("\n"), qr, barcodes, objects, v: VERSION, ..Default::default() })
    })
}

#[cfg(test)]
mod tests {
    /// Run with: OCR_SAMPLE=/path/to/image cargo test ocr_smoke -- --ignored --nocapture
    /// Prints only counts, never the recognized text.
    #[test]
    #[ignore]
    fn ocr_smoke() {
        let p = std::env::var("OCR_SAMPLE").expect("set OCR_SAMPLE");
        let t = std::time::Instant::now();
        let r = super::recognize(std::path::Path::new(&p)).expect("ocr failed");
        let hangul = r.text.chars().filter(|c| ('가'..='힣').contains(c)).count();
        println!(
            "lines={} chars={} hangul_chars={} qr={} took={:?}",
            r.text.lines().count(),
            r.text.chars().count(),
            hangul,
            r.qr.len(),
            t.elapsed()
        );
        assert!(!r.text.is_empty());
    }
}

#[cfg(test)]
mod object_tests {
    use super::*;

    #[test]
    fn merges_keeps_strongest_and_caps() {
        let mut m = HashMap::new();
        for i in 0..30 {
            m.insert(format!("label{i}"), 0.2 + i as f32 * 0.01);
        }
        m.insert("coffee".into(), 0.9);
        let v = finish_objects(m);
        assert_eq!(v.len(), OBJECT_MAX);
        assert_eq!(v[0], Obj { id: "coffee".into(), conf: 0.9 });
        assert!(v.windows(2).all(|w| w[0].conf >= w[1].conf), "sorted by confidence");
    }

    /// OBJ_SAMPLE=/path/to/image cargo test object_smoke -- --ignored --nocapture
    /// Prints only how many labels were found and the timing, not the labels.
    #[test]
    #[ignore]
    fn object_smoke() {
        let p = std::env::var("OBJ_SAMPLE").expect("set OBJ_SAMPLE");
        let t = std::time::Instant::now();
        let v = detect_objects(Path::new(&p)).expect("classify failed");
        println!("objects={} took={:?}", v.len(), t.elapsed());
    }
}

#[cfg(test)]
mod fp_tests {
    use super::*;

    /// FP_DIR=/folder cargo test fp_distances -- --ignored --nocapture
    /// Prints only the distance distribution (no image content).
    #[test]
    #[ignore]
    fn fp_distances() {
        let dir = std::env::var("FP_DIR").expect("set FP_DIR");
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png")).collect();
        files.sort();
        files.truncate(120);
        let t = std::time::Instant::now();
        let fps: Vec<Vec<f32>> = files.iter().filter_map(|p| feature_print(p)).collect();
        println!("prints={} dims={} took={:?}", fps.len(), fps.first().map_or(0, |v| v.len()), t.elapsed());
        // nearest-neighbour distance for every image
        let mut nn: Vec<f32> = (0..fps.len()).map(|i| (0..fps.len()).filter(|&j| j != i)
            .map(|j| fp_distance(&fps[i], &fps[j])).fold(f32::MAX, f32::min)).collect();
        nn.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f32| nn[((nn.len() - 1) as f32 * p) as usize];
        println!("nearest-neighbour distance: min={:.3} p10={:.3} p25={:.3} p50={:.3} p75={:.3} max={:.3}", nn[0], q(0.1), q(0.25), q(0.5), q(0.75), nn[nn.len()-1]);
        for th in [0.2f32, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8] {
            println!("  pairs under {th}: images with a neighbour = {}", nn.iter().filter(|d| **d <= th).count());
        }
    }
}
