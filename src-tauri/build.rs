use std::path::PathBuf;

fn main() {
    hid_plugin();
    tauri_build::build()
}

/// Embed the CPMX2077 HID Input plugin DLL (crates/hid-plugin) so the HID
/// input page can install it: from `CPMX_HID_PLUGIN_DLL` (CI builds the DLL
/// first and sets it), else the workspace's Windows release build when one
/// exists. Without either the app carries no plugin and the page says so.
fn hid_plugin() {
    println!("cargo:rerun-if-env-changed=CPMX_HID_PLUGIN_DLL");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("cpmx_hid_input.dll");
    let target = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../target");
    let found = std::env::var_os("CPMX_HID_PLUGIN_DLL").map(PathBuf::from).or_else(|| {
        ["x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"]
            .iter()
            .map(|t| target.join(t).join("release/cpmx_hid_input.dll"))
            .find(|p| p.is_file())
    });
    match found {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("CPMX_HID_PLUGIN_DLL {}: {e}", p.display()));
            assert!(bytes.starts_with(b"MZ"), "{} is not a Windows DLL", p.display());
            std::fs::write(&out, bytes).unwrap();
        }
        None => std::fs::write(&out, b"").unwrap(),
    }
}
