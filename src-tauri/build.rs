use std::{fs, process::Command};

fn main() {
    // Compile the Swift helper that talks to Apple's on-device model (Foundation Models is Swift-only).
    println!("cargo:rerun-if-changed=helper/describe.swift");
    let target = std::env::var("TARGET").unwrap_or_else(|_| "aarch64-apple-darwin".into());
    println!("cargo:rustc-env=BUILD_TARGET={target}");
    let out = format!("binaries/describe-helper-{target}");
    let _ = fs::create_dir_all("binaries");
    let swift_target = if target.starts_with("aarch64") { "arm64-apple-macos27.0" } else { "x86_64-apple-macos27.0" };
    let ok = Command::new("swiftc")
        .args(["-O", "-parse-as-library", "-target", swift_target, "helper/describe.swift", "-o", &out])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        // No Swift toolchain / older SDK: ship a stub that reports "unavailable" so the app still
        // builds and simply falls back to Vision classification.
        println!("cargo:warning=describe-helper not built; on-device picture understanding disabled");
        let _ = fs::write(&out, "#!/bin/sh\necho '{\"ready\":false,\"reason\":\"helper not built\"}'\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(0o755));
        }
    }
    tauri_build::build()
}
