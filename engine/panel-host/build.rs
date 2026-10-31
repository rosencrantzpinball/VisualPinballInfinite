//! vpi-panel finds libcef.so beside itself: where it's packaged, and where cef-dll-sys copies CEF's files
//! in a build (the target's own folder).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
