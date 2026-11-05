fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=packaging/windows/mdview.rc");
        println!("cargo:rerun-if-changed=assets/icon.ico");
        embed_resource::compile("packaging/windows/mdview.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
