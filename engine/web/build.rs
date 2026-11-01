//! Links libwinevbs into the browser's program: the static library `tools/web/libwinevbs.sh` builds
//! (into `vpx-build/wasm32/libwinevbs/build`), or the folder `VPI_WINEVBS_WASM` names.

fn main() {
    println!("cargo:rerun-if-env-changed=VPI_WINEVBS_WASM");
    if !std::env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|f| f.split(',').any(|f| f == "wasm")) {
        return;
    }
    let folder = std::env::var("VPI_WINEVBS_WASM").unwrap_or_else(|_| format!("{}/../../vpx-build/wasm32/libwinevbs/build", env!("CARGO_MANIFEST_DIR")));
    let library = std::path::Path::new(&folder).join("libwinevbs.a");
    if !library.is_file() {
        panic!("no {}: build it first (tools/web/libwinevbs.sh)", library.display());
    }
    println!("cargo:rerun-if-changed={}", library.display());
    println!("cargo:rustc-link-search=native={folder}");
    println!("cargo:rustc-link-lib=static=winevbs");
}
