use std::path::Path;
use std::process::Command;

/// Version « git » affichée par `--version` : tag ou hash court du commit.
/// La variable d'environnement SDR_UNIVERSAL_GIT la remplace (utile hors d'un
/// dépôt git, par exemple dans un paquet source).
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

    // Recalculer la version git quand un commit ou un tag est créé.
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
