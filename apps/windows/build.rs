fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    resource.set("ProductName", "Shadow Transfer");
    resource.set(
        "FileDescription",
        "Shadow Transfer - Transfer Without Limits.",
    );
    resource.set("LegalCopyright", "MIT License");
    resource.set("OriginalFilename", "ShadowTransfer.exe");
    resource.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    resource.set("FileVersion", env!("CARGO_PKG_VERSION"));
    if let Err(err) = resource.compile() {
        println!("cargo:warning=Windows resource compilation failed: {err}");
    }
}
