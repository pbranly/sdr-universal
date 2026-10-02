use std::path::Path;
use std::process::Command;

/// Git version shown by `--version`: tag or short commit hash.
/// The SDR_UNIVERSAL_GIT environment variable overrides it (useful outside a
/// git repository, for example in a source package).
fn git_version() -> String {
    if let Ok(value) = std::env::var("SDR_UNIVERSAL_GIT") {
        if !value.trim().is_empty() {
            return value.trim().to_string();
        }
    }

    Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SDR_UNIVERSAL_GIT");

    // Recompute the git version when a commit or a tag is created.
    for path in [".git/HEAD", ".git/logs/HEAD", ".git/packed-refs"] {
        if Path::new(path).exists() {
            println!("cargo:rerun-if-changed={}", path);
        }
    }

    println!("cargo:rustc-env=SDR_UNIVERSAL_GIT={}", git_version());
    println!(
        "cargo:rustc-env=SDR_UNIVERSAL_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string())
    );
}
