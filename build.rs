fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("app_icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("Warning: could not compile windows resource: {}", e);
        }
    }

    // Ensure dummy binary exists so `nova-installer` (which include_bytes! it)
    // can pass `cargo check` and `cargo clippy` without needing a prior release build.
    let target_bin = std::path::Path::new("target/release/my_rust_browser.exe");
    if !target_bin.exists() {
        if let Some(parent) = target_bin.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(target_bin, b"NOVA_PLACEHOLDER_BIN");
    }
}
