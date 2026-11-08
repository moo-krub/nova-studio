fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("app_icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("Warning: could not compile windows resource: {}", e);
        }
    }
}
