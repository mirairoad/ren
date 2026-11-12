fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/ren.ico");
        res.set("ProductName", "Ren IDE");
        res.set("FileDescription", "Ren IDE");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon could not be embedded: {}", e);
        }
    }
}
