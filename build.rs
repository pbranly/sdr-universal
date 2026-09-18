fn main() {
    println!("cargo:rustc-link-search=native=/usr/local/lib");
    println!("cargo:rustc-link-lib=dylib=sdrplay_api");

    println!("cargo:rerun-if-changed=/usr/local/include/sdrplay_api.h");
}
