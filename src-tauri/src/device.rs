//! Guess which Apple device a screenshot was taken on.
//!
//! iPhone / iPad / Apple Watch screenshots are always exactly the device's native screen
//! resolution, so an exact pixel-size match (either orientation) identifies the family.
//! Mac screenshots are recognized by macOS's own screen-capture tag on the file, or by a
//! full-display capture at a built-in Mac resolution. Cropped/edited images stay "other".

use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Device {
    /// "iphone" | "ipad" | "mac" | "watch" | "other"
    pub kind: &'static str,
    /// Model family, e.g. "iPhone 15 Pro · 16" (same in every language).
    pub model: Option<&'static str>,
}

/// (short side, long side, kind, model family)
const SCREENS: &[(u32, u32, &str, &str)] = &[
    // iPhone
    (640, 1136, "iphone", "iPhone SE (1st) · 5s"),
    (750, 1334, "iphone", "iPhone SE · 8 · 7 · 6s"),
    (1242, 2208, "iphone", "iPhone 8 Plus · 7 Plus"),
    (1080, 1920, "iphone", "iPhone 8 Plus · 7 Plus"),
    (1125, 2436, "iphone", "iPhone X · XS · 11 Pro"),
    (828, 1792, "iphone", "iPhone XR · 11"),
    (1242, 2688, "iphone", "iPhone XS Max · 11 Pro Max"),
    (1080, 2340, "iphone", "iPhone 12 mini · 13 mini"),
    (1170, 2532, "iphone", "iPhone 12 · 13 · 14 · 16e"),
    (1284, 2778, "iphone", "iPhone 12/13 Pro Max · 14 Plus"),
    (1179, 2556, "iphone", "iPhone 14 Pro · 15 · 15 Pro · 16"),
    (1290, 2796, "iphone", "iPhone 14 Pro Max · 15 Plus/Pro Max · 16 Plus"),
    (1206, 2622, "iphone", "iPhone 16 Pro · 17 · 17 Pro"),
    (1320, 2868, "iphone", "iPhone 16 Pro Max · 17 Pro Max"),
    (1260, 2736, "iphone", "iPhone Air"),
    // iPad
    (1536, 2048, "ipad", "iPad 9.7\" · mini 2–5 · Air 1–2"),
    (1620, 2160, "ipad", "iPad 10.2\" (7–9th)"),
    (1640, 2360, "ipad", "iPad 10.9\"/11\" · Air 4–5"),
    (1668, 2224, "ipad", "iPad Air 3 · Pro 10.5\""),
    (1668, 2388, "ipad", "iPad Pro 11\" · Air 11\""),
    (1668, 2420, "ipad", "iPad Pro 11\" (M4+)"),
    (2048, 2732, "ipad", "iPad Pro 12.9\" · Air 13\""),
    (2064, 2752, "ipad", "iPad Pro 13\" (M4+)"),
    (1488, 2266, "ipad", "iPad mini 6–7"),
    // Apple Watch
    (324, 394, "watch", "Apple Watch 40mm"),
    (368, 448, "watch", "Apple Watch 44mm"),
    (352, 430, "watch", "Apple Watch 41mm"),
    (396, 484, "watch", "Apple Watch 45mm"),
    (374, 446, "watch", "Apple Watch 42mm"),
    (416, 496, "watch", "Apple Watch 46mm"),
    (410, 502, "watch", "Apple Watch Ultra"),
    // Mac built-in displays (full-screen captures at default scaling)
    (1600, 2560, "mac", "MacBook Air/Pro 13\""),
    (1664, 2560, "mac", "MacBook Air 13\""),
    (1864, 2880, "mac", "MacBook Air 15\""),
    (1800, 2880, "mac", "MacBook Pro 15\""),
    (1920, 3072, "mac", "MacBook Pro 16\" (2019)"),
    (1964, 3024, "mac", "MacBook Pro 14\""),
    (2234, 3456, "mac", "MacBook Pro 16\""),
    (2520, 4480, "mac", "iMac 24\""),
    (2880, 5120, "mac", "iMac 27\" · Studio Display"),
    (3384, 6016, "mac", "Pro Display XDR"),
];

pub fn detect(width: u32, height: u32, mac_capture: bool) -> Device {
    let (short, long) = if width <= height { (width, height) } else { (height, width) };
    let hit = SCREENS.iter().find(|(s, l, _, _)| *s == short && *l == long);
    if mac_capture {
        // Tagged by macOS: definitely a Mac; name the display if it is a full-screen capture.
        let model = hit.filter(|h| h.2 == "mac").map(|h| h.3);
        return Device { kind: "mac", model };
    }
    match hit {
        Some(&(_, _, kind, model)) => Device { kind, model: Some(model) },
        None => Device { kind: "other", model: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_devices_by_exact_resolution() {
        assert_eq!(detect(1179, 2556, false).kind, "iphone");
        assert_eq!(detect(2556, 1179, false).kind, "iphone", "landscape");
        assert_eq!(detect(2048, 2732, false).kind, "ipad");
        assert_eq!(detect(396, 484, false).kind, "watch");
        assert_eq!(detect(3024, 1964, false), Device { kind: "mac", model: Some("MacBook Pro 14\"") });
        assert_eq!(detect(1179, 2000, false).kind, "other", "cropped/edited");
    }

    #[test]
    fn macos_capture_tag_wins() {
        assert_eq!(detect(812, 430, true), Device { kind: "mac", model: None }, "region capture");
        // an iPhone-sized window captured on a Mac is still a Mac screenshot
        assert_eq!(detect(1179, 2556, true), Device { kind: "mac", model: None });
    }
}
