fn main() {
    // `tauri-build` expects the `tauri` crate. Library tests disable the `gui`
    // feature so they do not link GTK.
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GUI");
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        tauri_build::build();
    }
}
