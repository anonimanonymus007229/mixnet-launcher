fn main() {
    println!("cargo:rerun-if-changed=../assets/branding/mixnet.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../assets/branding/mixnet.ico")
            .set("ProductName", "MixNet")
            .set("FileDescription", "MixNet — DDNet Mixin Launcher")
            .set("LegalCopyright", "MixNet contributors")
            .compile()
            .expect("Windows executable resources");
    }
}
