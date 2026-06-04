//! Build script: bake the current git commit into the binary so the in-app
//! updater can tell whether a newer commit is available upstream.

use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TUIUI_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
