//! Rule-based tagging of recognized screenshot text.
//!
//! - `temp`: throwaway screenshots (verification codes, QR codes, delivery tracking,
//!   payment/transfer confirmations, route directions, coupons/barcodes) — safe-to-delete candidates.
//! - `private`: screenshots containing personal/financial identifiers.
//!
//! Rules run on every load (not cached), so they can change without re-running OCR.

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Tag {
    /// "temp" | "private"
    pub kind: &'static str,
    /// Stable id (e.g. "otp", "card"); the UI translates it.
    pub label: &'static str,
}

const fn temp(label: &'static str) -> Tag {
    Tag { kind: "temp", label }
}
const fn private(label: &'static str) -> Tag {
    Tag { kind: "private", label }
}

/// Keyword lists are matched against text with all whitespace removed and lowercased,
/// so "인증 번호" and "인증번호" both hit; write keywords without spaces.
const OTP_WORDS: &[&str] = &[
    "인증번호", "인증코드", "확인코드", "보안코드", "일회용비밀번호", "로그인코드",
    "verificationcode", "verifycode", "securitycode", "one-timecode", "one-timepassword", "logincode",
];
const DELIVERY_WORDS: &[&str] = &[
    "배송조회", "운송장", "배송현황", "배송중", "배송완료", "배송출발", "배달완료", "배송예정",
    "trackingnumber", "outfordelivery",
];
const PAID_WORDS: &[&str] = &[
    "결제완료", "결제가완료", "결제되었습니다", "주문완료", "주문이완료", "송금완료", "이체완료",
    "입금완료", "보냈어요", "송금했어요", "paymentcomplete", "paymentsuccessful", "orderconfirmed",
];
const ROUTE_WORDS: &[&str] = &["길찾기", "경로안내", "도착예정"];
const COUPON_WORDS: &[&str] = &[
    "쿠폰", "교환권", "기프티콘", "모바일상품권", "상품권", "사용기한", "유효기간", "멤버십", "바코드",
    "coupon", "voucher", "giftcard", "membership",
];
const BANK_WORDS: &[&str] = &[
    "계좌", "국민", "신한", "우리은행", "하나은행", "농협", "기업은행", "카카오뱅크", "토스뱅크",
    "케이뱅크", "새마을", "우체국", "sc제일", "씨티", "수협", "부산은행", "대구은행", "ibk", "kb", "nh",
];

static CARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:\d{4}[ -]?){3}\d{4}\b").unwrap());
static RRN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{2})(\d{2})(\d{2})\s?-\s?([1-4])\d{6}\b").unwrap());
static ACCOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{2,6}-\d{2,6}-\d{2,8}(?:-\d{1,3})?\b|\b\d{10,14}\b").unwrap());
static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b01[016789][ -]?\d{3,4}[ -]?\d{4}\b").unwrap());
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").unwrap());
static PASSWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(비밀번호|패스워드|비번|password|passwd|pw)\s*[:：=]\s*\S{4,}").unwrap());
static PASSPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[MSRODG]\d{8}\b").unwrap());
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[가-힣]+(?:시|도)\s?[가-힣]+(?:구|군|시)\s?[가-힣0-9]+(?:로|길)\s?\d+").unwrap()
});
static OTP_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bOTP\b").unwrap());
/// A long barcode-style number: 12+ digits, possibly in space/dash-separated groups.
static CODE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{2,6}(?:[ -]?\d{2,6}){2,}\b").unwrap());
static OTP_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{4,8}\b").unwrap());

fn luhn(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() < 13 {
        return false;
    }
    let sum: u32 = ds
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| if i % 2 == 1 { let x = d * 2; if x > 9 { x - 9 } else { x } } else { d })
        .sum();
    sum % 10 == 0
}

/// Dummy numbers like 010-1111-1111 / 01000000000 (forms, test data) are not personal info.
fn is_real_phone(s: &str) -> bool {
    let digits: Vec<char> = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let tail: std::collections::BTreeSet<&char> = digits[3..].iter().collect();
    tail.len() > 2
}

fn any(compact: &str, words: &[&str]) -> bool {
    words.iter().any(|w| compact.contains(w))
}

/// True when the regex match is only part of a longer digit run (e.g. 16 of a barcode's 18 digits).
fn part_of_longer_number(text: &str, start: usize, end: usize) -> bool {
    let next = text[end..].trim_start_matches([' ', '-']).chars().next();
    let prev = text[..start].trim_end_matches([' ', '-']).chars().last();
    next.is_some_and(|c| c.is_ascii_digit()) || prev.is_some_and(|c| c.is_ascii_digit())
}

/// A long code number that is not simply a (Luhn-valid, 16-digit) card number.
fn long_code_number(text: &str) -> bool {
    CODE_NUMBER.find_iter(text).any(|m| {
        let digits: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
        digits.len() >= 12 && !(digits.len() == 16 && luhn(&digits))
    })
}

/// Phrases this app itself shows on screen (all UI languages). A screenshot of Clear Screenshot
/// contains our own labels ("사진 속: 영수증·티켓", tag names …), so its text must not be used to
/// classify it — otherwise the app would "find" whatever it displayed.
const OWN_APP_MARKERS: &[&str] = &[
    "clear screenshot", "사진 속:", "in the picture:", "写真の中:", "en la imagen:",
    "↑↓ 이동", "↑↓ move", "↑↓ 移動", "↑↓ mover", "⌘enter", "빠른 검토", "quick review",
    "모든 스크린샷", "all screenshots", "지워도 될 것", "safe to delete", "개인정보 포함",
    // buttons of the large view / toolbar (a capture of the preview has no search hints)
    "사진 앱 열기", "open photos", "텍스트 복사", "글자 복사", "copy text", "finder에서 보기", "show in finder",
    "분류 추가", "add category", "월별 폴더로 정리", "월별 정리", "이 분류 아님",
];

/// True when at least two of our own UI phrases appear (one alone could be coincidence).
pub fn is_own_app_capture(text: &str) -> bool {
    let low = text.to_lowercase();
    OWN_APP_MARKERS.iter().filter(|m| low.contains(*m)).count() >= 2
}

pub fn classify(text: &str, qr: &[String], barcodes: &[String]) -> Vec<Tag> {
    let mut tags = Vec::new();
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();

    // ---- throwaway ----
    // "OTP" must be its own word (whitespace-stripped text would find it inside "got perfect")
    // and is only trusted next to a 4–8 digit code. "승인번호" (receipt approval no.) is not an OTP.
    let otp_word = any(&compact, OTP_WORDS) || (OTP_WORD.is_match(text) && OTP_DIGITS.is_match(text));
    if otp_word {
        tags.push(temp("otp"));
    }
    if !qr.is_empty() {
        tags.push(temp("qr"));
    }
    if any(&compact, DELIVERY_WORDS) {
        tags.push(temp("delivery"));
    }
    if any(&compact, PAID_WORDS) {
        tags.push(temp("paid"));
    }
    // Coupons / membership / gift cards: a detected 1D barcode, or coupon wording next to a long code.
    if !barcodes.is_empty() || (any(&compact, COUPON_WORDS) && long_code_number(text)) {
        tags.push(temp("coupon"));
    }
    if any(&compact, ROUTE_WORDS) || (compact.contains("출발") && compact.contains("도착") && compact.contains("분")) {
        tags.push(temp("route"));
    }

    // ---- private ----
    if CARD
        .find_iter(text)
        .any(|m| luhn(m.as_str()) && !part_of_longer_number(text, m.start(), m.end()))
    {
        tags.push(private("card"));
    }
    if RRN.captures_iter(text).any(|c| {
        let m: u32 = c[2].parse().unwrap_or(0);
        let d: u32 = c[3].parse().unwrap_or(0);
        (1..=12).contains(&m) && (1..=31).contains(&d)
    }) {
        tags.push(private("rrn"));
    }
    if any(&compact, BANK_WORDS)
        && ACCOUNT.find_iter(text).any(|m| {
            let s = m.as_str();
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            // not a phone number, not a card number already counted, plausible length
            !digits.starts_with("01") && (10..=16).contains(&digits.len()) && !(digits.len() == 16 && luhn(&digits))
        })
    {
        tags.push(private("account"));
    }
    if PASSWORD.is_match(text) {
        tags.push(private("password"));
    }
    if PASSPORT.is_match(text) && (compact.contains("여권") || compact.contains("passport")) {
        tags.push(private("passport"));
    }
    if PHONE.find_iter(text).any(|m| is_real_phone(m.as_str())) {
        tags.push(private("phone"));
    }
    if EMAIL.is_match(text) {
        tags.push(private("email"));
    }
    if ADDRESS.is_match(text) {
        tags.push(private("address"));
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(text: &str) -> Vec<&'static str> {
        classify(text, &[], &[]).into_iter().map(|t| t.label).collect()
    }

    #[test]
    fn throwaway_rules() {
        assert_eq!(labels("[Web발신]\n인증 번호 [482913]를 입력해주세요"), vec!["otp"]);
        assert_eq!(labels("Your verification code is 123456"), vec!["otp"]);
        assert!(labels("OTP 기기 사용 안내").is_empty(), "bare 'otp' without a code is not enough");
        assert_eq!(labels("CJ대한통운\n배송 조회\n배송중"), vec!["delivery"]);
        assert_eq!(labels("결제가 완료되었습니다\n12,000원"), vec!["paid"]);
        assert_eq!(labels("강남역 출발 → 판교역 도착 32분"), vec!["route"]);
        assert_eq!(classify("", &["https://example.com".into()], &[])[0].label, "qr");
        assert!(labels("오늘 회의 안건 정리\n1. 디자인 리뷰").is_empty());
    }

    #[test]
    fn private_rules() {
        assert_eq!(labels("카드번호 4111 1111 1111 1111"), vec!["card"]); // Luhn-valid test number
        assert!(labels("주문번호 1234 5678 9012 3456").is_empty(), "Luhn-invalid 16 digits is not a card");
        assert_eq!(labels("주민등록번호 900101-1234567"), vec!["rrn"]);
        assert!(labels("코드 991399-1234567").is_empty(), "month 13 is not a valid RRN");
        assert_eq!(labels("신한 110-123-456789 홍길동"), vec!["account"]);
        assert!(labels("문의 110-123-456789").is_empty(), "digits without a bank context are not an account");
        assert_eq!(labels("wifi 비밀번호: abcd1234"), vec!["password"]);
        assert!(labels("비밀번호를 입력하세요").is_empty(), "a password field label alone is not a password");
        assert_eq!(labels("연락처 010-1234-5678"), vec!["phone"]);
        assert_eq!(labels("메일 someone@example.com"), vec!["email"]);
        assert_eq!(labels("서울특별시 강남구 테헤란로 123"), vec!["address"]);
        assert_eq!(labels("여권번호 M12345678"), vec!["passport"]);
    }

    #[test]
    fn coupons_and_barcodes() {
        let coupon = "26Y 상시혜택_사우나 30% (연 3회)\n9802 8907 9504 6237 90";
        let with_barcode: Vec<_> =
            classify(coupon, &[], &["VNBarcodeSymbologyCode128".into()]).into_iter().map(|t| t.label).collect();
        assert_eq!(with_barcode, vec!["coupon"], "barcode detected; 18 digits must not look like a card");
        assert!(labels(coupon).is_empty(), "no barcode and no coupon wording: not enough");
        assert_eq!(labels("스타벅스 아메리카노 교환권\n유효기간 2026.12.31\n1234 5678 9012"), vec!["coupon"]);
        assert!(labels("쿠폰함 (3)").is_empty(), "coupon wording without a code number");
        // a real 16-digit card number standing alone is still a card
        assert_eq!(labels("카드 4111 1111 1111 1111 유효기간 12/28"), vec!["card"]);
    }

    #[test]
    fn recognizes_captures_of_this_app() {
        assert!(is_own_app_capture("IMG_5147.PNG\n25.09.04 · iPhone 14 Pro · 사진 속: 영수증·티켓\n↑↓ 이동 · Enter 크게 보기"));
        assert!(is_own_app_capture("Clear Screenshot\nAll Screenshots 234"));
        assert!(!is_own_app_capture("사진 속: 바다 풍경 너무 예뻐요"), "one phrase alone is not enough");
        assert!(!is_own_app_capture("영수증\n합계 12,000원"));
        // a capture of the large view: only "사진 속:" plus its buttons
        assert!(is_own_app_capture("IMG_5147.PNG · 사진 속: 영수증·티켓\n← →\n글자 복사\n사진 앱 열기\n휴지통으로\n닫기"));
    }

    #[test]
    fn regressions_from_real_screenshots() {
        assert!(labels("현금영수증\n승인번호 123456789").is_empty(), "receipt approval number is not an OTP");
        assert!(labels("I got perfect scores").is_empty(), "'otp' across word boundary");
        assert_eq!(labels("OTP 482913"), vec!["otp"]);
        assert!(labels("관리자\n01011111111\n학부모\n01022222222").is_empty(), "dummy repeated-digit numbers");
        assert!(labels("010-0000-0000").is_empty());
        assert_eq!(labels("대표번호 010-2345-6789"), vec!["phone"]);
    }
}

#[cfg(test)]
mod real_data {
    /// Tally tag labels over the app's OCR cache. Prints counts only, never text.
    /// cargo test tally_cache -- --ignored --nocapture
    #[test]
    #[ignore]
    fn tally_cache() {
        let dir = dirs::cache_dir().unwrap().join("clear-screenshot/ocr-v1");
        let mut counts = std::collections::BTreeMap::new();
        let (mut n, mut with_text) = (0, 0);
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let Ok(s) = std::fs::read_to_string(e.path()) else { continue };
            let Ok(o) = serde_json::from_str::<crate::vision::Ocr>(&s) else { continue };
            n += 1;
            if !o.text.trim().is_empty() { with_text += 1; }
            for t in super::classify(&o.text, &o.qr, &o.barcodes) {
                *counts.entry(format!("{}:{}", t.kind, t.label)).or_insert(0) += 1;
            }
        }
        println!("analyzed={n} with_text={with_text}");
        for (k, v) in counts { println!("{k} = {v}"); }
    }
}

#[cfg(test)]
mod real_data_debug {
    use super::*;
    fn shape(s: &str) -> String {
        s.chars().map(|c| if c.is_ascii_digit() { 'd' } else { c }).collect()
    }
    /// Prints only digit *shapes*, distinct-digit counts and matched keywords — no real values.
    #[test]
    #[ignore]
    fn debug_hits() {
        let dir = dirs::cache_dir().unwrap().join("clear-screenshot/ocr-v1");
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let Ok(s) = std::fs::read_to_string(e.path()) else { continue };
            let Ok(o) = serde_json::from_str::<crate::vision::Ocr>(&s) else { continue };
            let compact: String = o.text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
            for m in PHONE.find_iter(&o.text) {
                let d: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
                let distinct = d[3..].chars().collect::<std::collections::BTreeSet<_>>().len();
                let before = o.text[..m.start()].chars().last().map(|c| if c.is_ascii_digit() { 'D' } else if c.is_alphabetic() { 'A' } else { c }).unwrap_or('^');
                let after = o.text[m.end()..].chars().next().map(|c| if c.is_ascii_digit() { 'D' } else if c.is_alphabetic() { 'A' } else { c }).unwrap_or('$');
                println!("PHONE shape={} distinct_tail={} before={before:?} after={after:?}", shape(m.as_str()), distinct);
            }
            for w in OTP_WORDS { if compact.contains(w) { println!("OTP word={w}"); } }
        }
    }
}
