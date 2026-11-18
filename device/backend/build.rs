// libvmx, the codec OMT sends, vendored from openmediatransport/libvmx
// (MIT, see vmx/LICENSE.txt). Only the Pi's 64-bit ARM is built; other
// machines get a backend whose OMT output says it can't run there.
fn main() {
    println!("cargo:rerun-if-changed=vmx/src");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64") {
        return;
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include("vmx/src")
        .file("vmx/src/vmxcodec.cpp")
        .file("vmx/src/vmxcodec_arm.cpp")
        // Upstream builds with clang's -fdeclspec. On ARM every use is a
        // 16-byte alignment, which g++ spells alignas
        .define("__declspec(x)", "alignas(16)")
        .flag_if_supported("-Wno-narrowing")
        .opt_level(3)
        // Third-party code: its warnings are upstream's business
        .warnings(false)
        .compile("vmx");
}
