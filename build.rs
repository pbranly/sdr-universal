fn main() {
    // La bibliothèque SDRplay n'est liée que si le backend SDRplay est compilé
    // (feature par défaut). `cargo build --no-default-features` fonctionne sur
    // une machine sans API SDRplay : seul le backend factice (--mock) est alors
    // disponible.
    if std::env::var("CARGO_FEATURE_SDRPLAY").is_ok() {
        println!("cargo:rustc-link-search=native=/usr/local/lib");
        println!("cargo:rustc-link-lib=dylib=sdrplay_api");

        println!("cargo:rerun-if-changed=/usr/local/include/sdrplay_api.h");
    }
}
