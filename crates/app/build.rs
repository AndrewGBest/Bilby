fn main() {
    // Element debug info lets the UI tests find elements on screen. It is only needed
    // by tests, so release builds leave it out.
    println!("cargo:rerun-if-changed=i18n/po");
    let debug = std::env::var("PROFILE").map(|p| p != "release").unwrap_or(true);
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug).with_bundled_translations("i18n/po");
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint UI failed to compile");
}
