//! Picture understanding with Apple's on-device model, via the bundled Swift `describe-helper`
//! (Foundation Models has no C/Objective-C API). The helper runs once and answers one image per
//! line; see helper/describe.swift for the protocol.

use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Mutex,
    },
    time::Duration,
};

/// What the on-device model saw. `categories` use the same keys as ui/objects.js.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiDesc {
    /// photo | illustration | app_screen | web_page | chat | document | receipt | map | chart | code | other
    #[serde(default)]
    pub kind: String,
    /// App/website the model recognized ("" if unsure). Used only when it names a known app.
    #[serde(default)]
    pub app: String,
    pub subjects: Vec<String>,
    pub categories: Vec<String>,
    pub summary: String,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    ready: Option<bool>,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    subjects: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    summary: String,
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

static HELPER: Mutex<Option<Helper>> = Mutex::new(None);
/// Set once the helper reported that the model is not available on this Mac.
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);
const PER_IMAGE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bump when the helper's prompt/output changes so stored descriptions are redone.
/// v2: screenshot kind first, photo topics only for real photos.
/// v3: also the app/website the screenshot was taken in.
pub const AI_VERSION: u32 = 3;

fn helper_path() -> PathBuf {
    // Bundled app: Tauri puts the sidecar next to the main executable (without the target suffix).
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("describe-helper"))) {
        if p.exists() {
            return p;
        }
    }
    // `cargo run` / tests: the binary build.rs produced.
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("binaries/describe-helper-{}", env!("BUILD_TARGET")))
}

fn start() -> Option<Helper> {
    let mut child = Command::new(helper_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let first = rx.recv_timeout(Duration::from_secs(20)).ok()?;
    let ready = serde_json::from_str::<Reply>(&first).ok().and_then(|r| r.ready).unwrap_or(false);
    if !ready {
        UNAVAILABLE.store(true, Ordering::Relaxed);
        let _ = child.kill();
        return None;
    }
    Some(Helper { child, stdin, lines: rx })
}

/// Whether picture understanding can run here (starts the helper on first use).
pub fn available() -> bool {
    if UNAVAILABLE.load(Ordering::Relaxed) {
        return false;
    }
    let mut guard = HELPER.lock().unwrap();
    if guard.is_none() {
        *guard = start();
    }
    guard.is_some()
}

/// Describe one image. None if the model is unavailable, refused, or timed out.
pub fn describe(image: &Path) -> Option<AiDesc> {
    if !available() {
        return None;
    }
    let mut guard = HELPER.lock().unwrap();
    let h = guard.as_mut()?;
    let sent = writeln!(h.stdin, "{}", image.display()).and_then(|_| h.stdin.flush());
    let reply = sent.ok().and_then(|_| h.lines.recv_timeout(PER_IMAGE_TIMEOUT).ok());
    let Some(line) = reply else {
        // Stuck or crashed: drop it; the next call starts a fresh helper.
        let _ = h.child.kill();
        *guard = None;
        return None;
    };
    let r: Reply = serde_json::from_str(&line).ok()?;
    r.ok.then(|| AiDesc { kind: r.kind, app: r.app, subjects: r.subjects, categories: r.categories, summary: r.summary })
}

#[cfg(test)]
mod tests {
    /// AI_SAMPLE=/path/to/image cargo test ai_smoke -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ai_smoke() {
        let p = std::env::var("AI_SAMPLE").expect("set AI_SAMPLE");
        println!("available={}", super::available());
        let t = std::time::Instant::now();
        let d = super::describe(std::path::Path::new(&p));
        println!("categories={:?} subjects={:?} took={:?}", d.as_ref().map(|d| &d.categories), d.as_ref().map(|d| &d.subjects), t.elapsed());
        let t = std::time::Instant::now();
        let again = super::describe(std::path::Path::new(&p));
        println!("second call (helper reused) took={:?} same={}", t.elapsed(), again == d);
    }
}
