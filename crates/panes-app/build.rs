#[cfg(target_os = "windows")]
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/windows/Panes.exe.manifest");

    println!("cargo:rerun-if-changed={}", manifest.display());

    winresource::WindowsResource::new()
        .set_manifest_file(
            manifest
                .to_str()
                .expect("Windows manifest path must be valid UTF-8"),
        )
        .compile()
        .expect("failed to compile Windows executable resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
